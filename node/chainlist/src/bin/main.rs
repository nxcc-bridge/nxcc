use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use alloy_provider::{Provider, ProviderBuilder};
use alloy_rpc_types::BlockNumberOrTag;
use futures::stream::{self, StreamExt};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use nxcc_chainlist::types::{Chain, RpcEndpoints, SourceChain};
use reqwest::Client;
use tokio::sync::Mutex;
use tracing::info;
use url::Url;

const CHAINS_URL: &str = "https://chainlist.org/rpcs.json";
const CONCURRENCY_LIMIT: usize = 100;
const RPC_TIMEOUT: Duration = Duration::from_secs(20);
const OUTPUT_PATH: &str = "src/chains.json";

const BLOCK_TIME_SAMPLE_SIZE: u64 = 20;
const BLOCK_FETCH_CONCURRENCY: usize = 10;
const CHAIN_PROCESSING_CONCURRENCY: usize = 25;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Protocol {
    Https,
    Wss,
}

#[derive(Debug)]
struct RpcTask {
    chain_id: u64,
    chain_name: String,
    url: Url,
    tracking: Option<String>,
}

#[derive(Clone, Debug)]
struct RpcTestResult {
    chain_id: u64,
    chain_name: String,
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

    let start_time = Instant::now();
    info!("Starting chainlist generation...");

    let client = Client::builder().timeout(Duration::from_secs(60)).build()?;

    let source_chains = fetch_source_chains(&client).await?;
    let tasks = prepare_rpc_tasks(source_chains);
    let results = execute_rpc_tests(tasks).await;
    let final_chains = process_results(results).await?;

    write_output(&final_chains)?;

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

fn prepare_rpc_tasks(source_chains: Vec<SourceChain>) -> Vec<RpcTask> {
    let mut tasks = Vec::new();
    let mut seen_urls = HashSet::new();

    for chain in source_chains {
        for rpc in chain.rpc {
            if rpc.url.contains("${") || !(rpc.url.starts_with("http") || rpc.url.starts_with("ws"))
            {
                continue;
            }

            if let Ok(url) = Url::parse(&rpc.url) {
                let mut add_task_if_new = |task_url: Url| {
                    if seen_urls.insert((chain.chain_id, task_url.to_string())) {
                        tasks.push(RpcTask {
                            chain_id: chain.chain_id,
                            chain_name: chain.name.clone(),
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
    info!("Prepared {} unique RPC URLs to test.", tasks.len());
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

async fn execute_rpc_tests(tasks: Vec<RpcTask>) -> Vec<RpcTestResult> {
    let m = MultiProgress::new();
    let pb_style = ProgressStyle::default_bar()
        .template(
            "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({percent}%) \
             {msg}",
        )
        .unwrap()
        .progress_chars("#>-");
    let pb = m.add(ProgressBar::new(tasks.len() as u64));
    pb.set_style(pb_style);
    pb.set_message("Testing RPCs...");

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
                    chain_id: task.chain_id,
                    chain_name: task.chain_name,
                    url: task.url,
                    protocol,
                    block_number,
                    tracking: task.tracking,
                });
                pb_clone.inc(1);
            }
        })
        .await;

    pb.finish_with_message("All RPCs tested.");
    Arc::try_unwrap(results)
        .expect("Mutex still has multiple owners")
        .into_inner()
}

async fn test_rpc(url: &Url) -> eyre::Result<u64> {
    let provider = match url.scheme() {
        "http" | "https" => ProviderBuilder::new().connect_http(url.clone()),
        "ws" | "wss" => {
            let fut =
                ProviderBuilder::new().connect_ws(alloy_provider::WsConnect::new(url.clone()));
            tokio::time::timeout(RPC_TIMEOUT, fut).await??
        }
        _ => return Err(eyre::eyre!("Unsupported scheme")),
    };

    let fut = provider.get_block_number();
    let block_number = tokio::time::timeout(RPC_TIMEOUT, fut).await??;

    Ok(block_number)
}

async fn calculate_block_time_stats_for_chain(
    chain_id: u64,
    successful_rpcs: &[(RpcTestResult, u64)],
) -> (Option<u64>, Option<f64>) {
    let best_rpc = match successful_rpcs.iter().max_by_key(|(_, block)| *block) {
        Some((r, _)) => r,
        None => return (None, None),
    };

    let (avg, var) = match best_rpc.protocol {
        Protocol::Https => {
            let provider = ProviderBuilder::new().connect_http(best_rpc.url.clone());
            try_calculate_stats(&provider).await
        }
        Protocol::Wss => {
            let ws_fut = ProviderBuilder::new()
                .connect_ws(alloy_provider::WsConnect::new(best_rpc.url.clone()));
            match tokio::time::timeout(RPC_TIMEOUT, ws_fut).await {
                Ok(Ok(provider)) => try_calculate_stats(&provider).await,
                Ok(Err(e)) => {
                    tracing::debug!(
                        "Failed to create WSS provider for chain {}: {}",
                        chain_id,
                        e
                    );
                    (None, None)
                }
                Err(_) => {
                    tracing::debug!("WSS provider connection timed out for chain {}", chain_id);
                    (None, None)
                }
            }
        }
    };

    if avg.is_some() {
        info!(
            "Calculated block time for chain {} (avg: {} ms)",
            chain_id,
            avg.unwrap()
        );
    }

    (avg, var)
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

async fn process_results(results: Vec<RpcTestResult>) -> eyre::Result<Vec<Chain>> {
    info!("Processing {} test results...", results.len());
    let mut chains_by_id: HashMap<u64, (String, Vec<RpcTestResult>)> = HashMap::new();
    for res in results {
        chains_by_id
            .entry(res.chain_id)
            .or_insert_with(|| (res.chain_name.clone(), Vec::new()))
            .1
            .push(res);
    }

    let mut final_chains = Vec::new();
    let mut chain_stream = stream::iter(chains_by_id)
        .map(|(chain_id, (name, results))| async move {
            let successful_results: Vec<_> = results
                .into_iter()
                .filter_map(|r| r.block_number.map(|block| (r, block)))
                .collect();

            if successful_results.is_empty() {
                tracing::warn!(
                    "Dropping chain {} ({}): No successful RPCs.",
                    chain_id,
                    name
                );
                return None;
            }

            let (best_rpc_from_initial_pass, initial_max_block) = successful_results
                .iter()
                .max_by_key(|(_, block)| *block)
                .unwrap();

            let definitive_latest_block = match test_rpc(&best_rpc_from_initial_pass.url).await {
                Ok(block) => block,
                Err(e) => {
                    tracing::warn!(
                        "Could not get definitive latest block for chain {}: {}. Falling back to \
                         initial max block.",
                        chain_id,
                        e
                    );
                    *initial_max_block
                }
            };

            let (average_block_time_ms, block_time_variance_ms) =
                calculate_block_time_stats_for_chain(chain_id, &successful_results).await;

            let mut scored_rpcs: Vec<ScoredRpc> = successful_results
                .into_iter()
                .map(|(r, block)| {
                    let up_to_date_score = if block >= definitive_latest_block.saturating_sub(5) {
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
                .filter(|r| r.score > 0.0)
                .collect();

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

            if !rpcs.is_empty() {
                Some(Chain {
                    chain_id,
                    name,
                    rpcs,
                    average_block_time_ms,
                    block_time_variance_ms,
                })
            } else {
                tracing::warn!(
                    "Dropping chain {} ({}): No RPCs passed the scoring filter.",
                    chain_id,
                    name
                );
                None
            }
        })
        .buffer_unordered(CHAIN_PROCESSING_CONCURRENCY);

    while let Some(Some(chain)) = chain_stream.next().await {
        final_chains.push(chain);
    }

    final_chains.sort_by_key(|c| c.chain_id);
    info!(
        "Final list contains {} chains with valid RPCs.",
        final_chains.len()
    );
    Ok(final_chains)
}

fn write_output(chains: &[Chain]) -> eyre::Result<()> {
    info!("Writing output to {}", OUTPUT_PATH);
    let output_path = Path::new(OUTPUT_PATH);
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json_str = serde_json::to_string_pretty(chains)?;
    fs::write(output_path, json_str)?;
    Ok(())
}
