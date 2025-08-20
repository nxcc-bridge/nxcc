use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

use alloy_provider::{Provider, ProviderBuilder};
use alloy_rpc_types::BlockNumberOrTag;
use clap::Parser;
use futures::stream::{self, StreamExt};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use nxcc_chainlist::types::{Chain as LibraryChain, RpcEndpoints, SourceChain};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::info;
use url::Url;

const CHAINS_URL: &str = "https://chainlist.org/rpcs.json";
const CONCURRENCY_LIMIT: usize = 100;
const RPC_TIMEOUT: Duration = Duration::from_millis(500);
const OUTPUT_DIR: &str = "chains";

const BLOCK_TIME_SAMPLE_SIZE: u64 = 20;
const BLOCK_FETCH_CONCURRENCY: usize = 10;

/// Represents a single chain with its metadata and curated RPC endpoints,
/// including the last_updated timestamp for internal binary use.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedChain {
    pub name: String,
    pub chain_id: u64,
    pub rpcs: RpcEndpoints,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_block_time_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_time_variance_ms: Option<f64>,
    /// Unix timestamp of when this chain's data was last updated.
    pub last_updated: u64,
}

/// A utility to generate a curated list of reliable and performant RPC endpoints for various chains.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Optional path to a local rpcs.json file.
    /// If not provided, it will be downloaded from chainlist.org.
    #[arg(long, short = 'f')]
    file: Option<String>,

    /// Skip checking chains that have been updated within this many hours.
    #[arg(long, default_value = "72")]
    freshness_hours: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Protocol {
    Https,
    Wss,
}

#[derive(Debug)]
struct RpcTask {
    url: Url,
    tracking: Option<String>,
}

#[derive(Clone, Debug)]
struct RpcTestResult {
    url: Url,
    protocol: Protocol,
    block_number: Option<u64>,
    tracking: Option<String>,
}

#[derive(Debug)]
struct ScoredRpc {
    url: String,
    protocol: Protocol,
    score: f64,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // Parse command-line arguments
    let cli = Cli::parse();
    let freshness_duration = Duration::from_secs(cli.freshness_hours * 3600);

    let start_time = Instant::now();
    info!("Starting chainlist generation...");

    let client = Client::builder().timeout(Duration::from_secs(60)).build()?;

    // 1. Extract all chains available.
    // If a file path is provided, use it. Otherwise, download from the URL.
    let source_chains: Vec<SourceChain> = if let Some(path_str) = cli.file {
        info!("Reading chain data from local file: {}", path_str);
        let path = Path::new(&path_str);
        if !path.exists() {
            return Err(eyre::eyre!(
                "Provided file path does not exist: {}",
                path_str
            ));
        }
        let content = fs::read_to_string(path)?;
        serde_json::from_str(&content)?
    } else {
        fetch_source_chains(&client).await?
    };

    let mut chains_by_id: HashMap<u64, Vec<SourceChain>> = HashMap::new();
    for chain in source_chains {
        chains_by_id.entry(chain.chain_id).or_default().push(chain);
    }
    // sort them
    let mut sorted_chain_ids: Vec<u64> = chains_by_id.keys().cloned().collect();
    sorted_chain_ids.sort();

    // Prepare output directory
    fs::create_dir_all(OUTPUT_DIR)?;
    info!("Output will be written to the '{}' directory.", OUTPUT_DIR);

    let mut oldest_last_updated = u64::MAX;
    if Path::new(OUTPUT_DIR).exists() {
        for entry in fs::read_dir(OUTPUT_DIR)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file()
                && path.extension().and_then(|s| s.to_str()) == Some("json")
                && let Ok(content) = fs::read_to_string(&path)
                && let Ok(existing_chain) = serde_json::from_str::<GeneratedChain>(&content)
            {
                oldest_last_updated = oldest_last_updated.min(existing_chain.last_updated);
            }
        }
    }

    let are_all_chains_fresh = if oldest_last_updated == u64::MAX {
        info!("No existing chain files found. Performing a full scan.");
        false
    } else {
        let now_ts = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs();
        let age = Duration::from_secs(now_ts.saturating_sub(oldest_last_updated));
        if age < freshness_duration {
            info!(
                "All chain files are fresh (oldest is {:?} old). Will only check for new chains.",
                age
            );
            true
        } else {
            info!(
                "Stale chain files detected (oldest is {:?} old). Performing a full refresh.",
                age
            );
            false
        }
    };

    let m = MultiProgress::new();
    let main_pb_style = ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] Chains: {pos}/{len} | {wide_msg}")
        .unwrap()
        .progress_chars("#>-");
    let main_pb = m.add(ProgressBar::new(sorted_chain_ids.len() as u64));
    main_pb.set_style(main_pb_style);

    // 2. for each chain
    for &chain_id in &sorted_chain_ids {
        let chain_file_path_str = format!("{}/{}.json", OUTPUT_DIR, chain_id);
        let chain_file_path = Path::new(&chain_file_path_str);

        if are_all_chains_fresh && chain_file_path.exists() {
            main_pb.inc(1);
            continue;
        }

        let sources = chains_by_id.get(&chain_id).unwrap();
        let chain_name = &sources[0].name;
        main_pb.set_message(format!("Chain {} ({})", chain_id, chain_name));

        // 2a. test all of its RPCs with maximum concurrency
        let tasks = prepare_rpc_tasks_for_chain(sources);
        if tasks.is_empty() {
            info!(
                "Chain {} ({}) has no valid RPC URLs to test. Skipping.",
                chain_id, chain_name
            );
            main_pb.inc(1);
            continue;
        }

        let rpc_pb = m.add(ProgressBar::new(tasks.len() as u64));
        let rpc_pb_style = ProgressStyle::default_bar()
            .template("  └─ RPCs for {msg}: [{bar:30.cyan/blue}] {pos}/{len}")
            .unwrap()
            .progress_chars("#>-");
        rpc_pb.set_style(rpc_pb_style);
        rpc_pb.set_message(format!("{}", chain_id));

        let results = execute_rpc_tests(tasks, rpc_pb.clone()).await;
        rpc_pb.finish_and_clear();

        // 2b. test the block time and block time variance
        let chain_to_write = if let Some(chain) =
            process_single_chain_results(chain_id, chain_name.clone(), results).await
        {
            chain
        } else {
            info!(
                "No valid RPCs found for chain {} ({}). Writing empty file to prevent re-checks.",
                chain_id, chain_name
            );
            let last_updated = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)?
                .as_secs();
            GeneratedChain {
                chain_id,
                name: chain_name.clone(),
                rpcs: RpcEndpoints::default(),
                average_block_time_ms: None,
                block_time_variance_ms: None,
                last_updated,
            }
        };
        write_chain_to_file(&chain_to_write)?;
        main_pb.inc(1);
    }

    main_pb.finish_with_message("All chains processed.");

    collect_chains_to_file().await?;

    info!(
        "Chainlist generation finished in {:.2?}.",
        start_time.elapsed()
    );
    Ok(())
}

async fn fetch_source_chains(client: &Client) -> eyre::Result<Vec<SourceChain>> {
    info!("Fetching chain data from {}", CHAINS_URL);
    let chains = client.get(CHAINS_URL).send().await?.json().await?;
    Ok(chains)
}

fn prepare_rpc_tasks_for_chain(source_chains: &[SourceChain]) -> Vec<RpcTask> {
    let mut tasks = Vec::new();
    let mut seen_urls = HashSet::new();

    for chain in source_chains {
        for rpc in &chain.rpc {
            if rpc.url.contains("${") || !(rpc.url.starts_with("http") || rpc.url.starts_with("ws"))
            {
                continue;
            }

            if let Ok(url) = Url::parse(&rpc.url) {
                let mut add_task_if_new = |task_url: Url| {
                    if seen_urls.insert((chain.chain_id, task_url.to_string())) {
                        tasks.push(RpcTask {
                            url: task_url,
                            tracking: rpc.tracking.clone(),
                        });
                    }
                };

                add_task_if_new(url.clone());

                if let Some(ws_url) = http_to_ws_url(&url) {
                    add_task_if_new(ws_url);
                }
            }
        }
    }
    tasks
}

fn http_to_ws_url(url: &Url) -> Option<Url> {
    let new_scheme = match url.scheme() {
        "http" => "ws",
        "https" => "wss",
        _ => return None,
    };
    let mut ws_url = url.clone();
    ws_url.set_scheme(new_scheme).ok()?;
    Some(ws_url)
}

async fn execute_rpc_tests(tasks: Vec<RpcTask>, pb: ProgressBar) -> Vec<RpcTestResult> {
    let results = Arc::new(Mutex::new(Vec::new()));

    stream::iter(tasks)
        .for_each_concurrent(CONCURRENCY_LIMIT, |task| {
            let results_clone = Arc::clone(&results);
            let pb_clone = pb.clone();
            async move {
                let result = test_rpc(&task.url).await;
                let block_number = match result {
                    Ok(bn) => Some(bn),
                    Err(e) => {
                        tracing::debug!("RPC test failed for {}: {}", task.url, e);
                        None
                    }
                };

                let protocol = match task.url.scheme() {
                    "https" | "http" => Protocol::Https,
                    "wss" | "ws" => Protocol::Wss,
                    _ => return,
                };
                results_clone.lock().await.push(RpcTestResult {
                    url: task.url,
                    protocol,
                    block_number,
                    tracking: task.tracking,
                });
                pb_clone.inc(1);
            }
        })
        .await;

    Arc::try_unwrap(results)
        .expect("Mutex still has multiple owners")
        .into_inner()
}

async fn test_rpc(url: &Url) -> eyre::Result<u64> {
    let result: Result<u64, eyre::Report> = (async {
        // Build the provider. This step can fail if the WS connection times out.
        let provider = match url.scheme() {
            "http" | "https" => ProviderBuilder::new().connect_http(url.clone()),
            "ws" | "wss" => {
                let connect_fut =
                    ProviderBuilder::new().connect_ws(alloy_provider::WsConnect::new(url.clone()));
                // The WS connection itself needs a timeout.
                tokio::time::timeout(RPC_TIMEOUT, connect_fut)
                    .await
                    .map_err(|_| eyre::eyre!("WSS connection timed out"))??
            }
            _ => return Err(eyre::eyre!("Unsupported scheme: {}", url.scheme())),
        };

        // Make the RPC call. This future is what we want to time out.
        let rpc_call_fut = provider.get_block_number();
        let block_number = tokio::time::timeout(RPC_TIMEOUT, rpc_call_fut)
            .await
            .map_err(|_| eyre::eyre!("RPC call timed out"))??;

        Ok(block_number)
    })
    .await;

    match result {
        Ok(block_number) => Ok(block_number),
        Err(e) => {
            let error_string = format!("{:?}", e).to_lowercase();
            if error_string.contains("429") || error_string.contains("too many requests") {
                // This is a "successful" failure (we know the RPC is alive but rate limited)
                tracing::debug!(
                    "RPC {} is rate-limiting (429), counting as low-quality success.",
                    url
                );
                Ok(1)
            } else {
                // This is a hard failure, log it with full details.
                tracing::warn!("RPC test failed for {url}:\n{e:#}");
                Err(e)
            }
        }
    }
}

/// Iterates through successful RPCs for a chain, from best to worst,
/// attempting to calculate block time stats. This is more resilient than
/// relying on a single RPC.
async fn calculate_block_time_stats_for_chain(
    chain_id: u64,
    successful_rpcs: &[(RpcTestResult, u64)],
) -> (Option<u64>, Option<f64>) {
    if successful_rpcs.is_empty() {
        return (None, None);
    }

    // Sort RPCs by block number, descending, to try the best ones first.
    let mut sorted_rpcs = successful_rpcs.to_vec();
    sorted_rpcs.sort_by_key(|(_, block)| std::cmp::Reverse(*block));

    for (rpc_result, _) in &sorted_rpcs {
        tracing::debug!(
            "Attempting block time calculation for chain {} with RPC: {}",
            chain_id,
            rpc_result.url
        );

        // Create a provider. This can fail for WSS due to connection issues or timeouts.
        let provider = match rpc_result.protocol {
            Protocol::Https => ProviderBuilder::new().connect_http(rpc_result.url.clone()),
            Protocol::Wss => {
                let fut = ProviderBuilder::new()
                    .connect_ws(alloy_provider::WsConnect::new(rpc_result.url.clone()));
                match tokio::time::timeout(RPC_TIMEOUT, fut).await {
                    Ok(Ok(provider)) => provider,
                    Ok(Err(e)) => {
                        tracing::debug!(
                            "Failed to create WSS provider for chain {} with RPC {}: {}. Trying \
                             next.",
                            chain_id,
                            rpc_result.url,
                            e
                        );
                        continue;
                    }
                    Err(_) => {
                        tracing::debug!(
                            "WSS provider connection timed out for chain {} with RPC {}. Trying \
                             next.",
                            chain_id,
                            rpc_result.url
                        );
                        continue;
                    }
                }
            }
        };

        // Try to calculate stats with the connected provider.
        match try_calculate_stats(&provider).await {
            (Some(avg), Some(var)) => {
                info!(
                    "Successfully calculated block time for chain {} using {}: avg {} ms",
                    chain_id, rpc_result.url, avg
                );
                return (Some(avg), Some(var)); // Success, we're done for this chain.
            }
            _ => {
                // This attempt failed, loop will try the next RPC.
                tracing::debug!(
                    "Block time stat calculation failed for chain {} with RPC {}. Trying next.",
                    chain_id,
                    rpc_result.url
                );
            }
        }
    }

    // If we've looped through all successful RPCs and none worked for stat calculation.
    tracing::warn!(
        "Could not calculate block time for chain {}: all candidate RPCs failed the test.",
        chain_id
    );
    (None, None)
}

async fn try_calculate_stats<P: Provider + Send + Sync>(
    provider: &P,
) -> (Option<u64>, Option<f64>) {
    match calculate_block_time_stats(provider).await {
        Ok((avg, var)) => (Some(avg), Some(var)),
        Err(e) => {
            tracing::debug!("Could not calculate block time: {}", e);
            (None, None)
        }
    }
}

async fn calculate_block_time_stats<P: Provider>(provider: &P) -> eyre::Result<(u64, f64)> {
    let latest_block_number = provider.get_block_number().await?;

    if latest_block_number < BLOCK_TIME_SAMPLE_SIZE {
        return Err(eyre::eyre!(
            "Not enough blocks to sample (height: {}, sample: {})",
            latest_block_number,
            BLOCK_TIME_SAMPLE_SIZE
        ));
    }

    let start_block = latest_block_number - BLOCK_TIME_SAMPLE_SIZE;
    let block_numbers_to_fetch = start_block..=latest_block_number;

    let timestamps: Vec<u64> = stream::iter(block_numbers_to_fetch)
        .map(|n| async move {
            provider
                .get_block_by_number(BlockNumberOrTag::Number(n))
                .await
                .ok()
                .flatten()
                .map(|b| b.header.timestamp)
        })
        .buffer_unordered(BLOCK_FETCH_CONCURRENCY)
        .filter_map(|t| async move { t })
        .collect::<Vec<u64>>()
        .await;

    let mut sorted_timestamps = timestamps;
    sorted_timestamps.sort_unstable();

    if sorted_timestamps.len() < 2 {
        return Err(eyre::eyre!(
            "Could not retrieve enough block timestamps (got {})",
            sorted_timestamps.len()
        ));
    }

    let deltas: Vec<u64> = sorted_timestamps
        .windows(2)
        .map(|w| w[1].saturating_sub(w[0]))
        .filter(|&d| d > 0)
        .collect();

    if deltas.is_empty() {
        return Err(eyre::eyre!("Could not calculate any valid time deltas"));
    }

    let count = deltas.len() as f64;
    let sum: u64 = deltas.iter().sum();
    let average_secs = sum as f64 / count;

    let variance_secs = deltas
        .iter()
        .map(|&delta| {
            let diff = delta as f64 - average_secs;
            diff * diff
        })
        .sum::<f64>()
        / count;

    let average_ms = (average_secs * 1000.0).round() as u64;
    let variance_ms = variance_secs * 1_000_000.0;

    Ok((average_ms, variance_ms))
}

async fn process_single_chain_results(
    chain_id: u64,
    name: String,
    results: Vec<RpcTestResult>,
) -> Option<GeneratedChain> {
    let successful_results: Vec<_> = results
        .into_iter()
        .filter_map(|r| r.block_number.map(|block| (r, block)))
        .collect();

    if successful_results.is_empty() {
        tracing::debug!("Chain {} ({}) has no successful RPCs.", chain_id, name);
        return None;
    }

    // Use the max block from the initial pass as the single source of truth.
    let (_best_rpc, max_block_in_pass) =
        successful_results.iter().max_by_key(|(_, block)| *block)?;

    let (average_block_time_ms, block_time_variance_ms) =
        calculate_block_time_stats_for_chain(chain_id, &successful_results).await;

    let mut scored_rpcs: Vec<ScoredRpc> = successful_results
        .iter()
        .map(|(r, block)| {
            // Score against the max block from the initial pass.
            let up_to_date_score = if *block >= max_block_in_pass.saturating_sub(5) {
                1000.0
            } else {
                0.0
            };

            let privacy_score = match r.tracking.as_deref() {
                Some("none") => 3.0,
                Some("limited") => 2.0,
                Some("unspecified") | None => 1.0,
                Some("yes") => 0.0,
                _ => 0.0,
            };

            let score = up_to_date_score + privacy_score;
            ScoredRpc {
                url: r.url.to_string(),
                protocol: r.protocol,
                score,
            }
        })
        .filter(|r| {
            if r.score <= 0.0 {
                tracing::debug!(
                    "Filtering out url {} for chain {} due to low score",
                    r.url,
                    chain_id
                );
            }
            r.score > 0.0
        })
        .collect();

    if scored_rpcs.is_empty() {
        return None;
    }

    scored_rpcs.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.url.cmp(&b.url))
    });

    let mut rpcs = RpcEndpoints::default();
    for rpc in scored_rpcs {
        match rpc.protocol {
            Protocol::Https => rpcs.https.push(rpc.url),
            Protocol::Wss => rpcs.wss.push(rpc.url),
        }
    }

    // Get the current Unix timestamp to mark when this data was generated.
    let last_updated = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("Time went backwards, unable to get current timestamp.")
        .as_secs();

    Some(GeneratedChain {
        chain_id,
        name,
        rpcs,
        average_block_time_ms,
        block_time_variance_ms,
        last_updated,
    })
}

fn write_chain_to_file(chain: &GeneratedChain) -> eyre::Result<()> {
    let path_str = format!("{}/{}.json", OUTPUT_DIR, chain.chain_id);
    let output_path = Path::new(&path_str);

    let json_str = serde_json::to_string_pretty(chain)?;
    fs::write(output_path, json_str)?;
    Ok(())
}

async fn collect_chains_to_file() -> eyre::Result<()> {
    info!("Collecting all chains into src/chains.json...");

    let mut all_chains: Vec<LibraryChain> = Vec::new();

    for entry in fs::read_dir(OUTPUT_DIR)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("json") {
            let content = fs::read_to_string(&path)?;
            let chain: GeneratedChain = match serde_json::from_str(&content) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("Failed to parse chain file {:?}: {}. Skipping.", path, e);
                    continue;
                }
            };

            // Convert GeneratedChain to LibraryChain (which does not have last_updated)
            all_chains.push(LibraryChain {
                chain_id: chain.chain_id,
                name: chain.name,
                rpcs: chain.rpcs,
                average_block_time_ms: chain.average_block_time_ms,
                block_time_variance_ms: chain.block_time_variance_ms,
            });
        }
    }

    all_chains.sort_by_key(|c| c.chain_id);

    let final_json_path = Path::new("src/chains.json");
    let json_str = serde_json::to_string_pretty(&all_chains)?;
    fs::write(final_json_path, json_str)?;

    info!(
        "Successfully wrote {} chains to src/chains.json",
        all_chains.len()
    );

    Ok(())
}
