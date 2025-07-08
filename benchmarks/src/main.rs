use std::time::{Duration, Instant};

use alloy_primitives::U256;
use anyhow::Result;
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use nxcc_interface::{
    proto::daemon::work_order_client::WorkOrderClient,
    types::{WorkerEvent, WorkerEventKind},
};
use tonic::transport::Channel;
use tracing::info;

mod utils;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long, default_value = "http://localhost:50051")]
    node_grpc_addr: String,
    #[arg(long, default_value = "http://localhost:8545")]
    anvil_rpc_url: String,
    #[command(subcommand)]
    command: Benchmark,
}

#[derive(Subcommand, Debug)]
enum Benchmark {
    /// Benchmark idle worker capacity
    Idle,
    /// Benchmark CPU-bound active worker capacity
    Cpu,
    /// Benchmark IO-bound active worker capacity
    Io,
    /// Benchmark realistic (CPU + IO) active worker capacity
    Realistic,
    /// Benchmark Web3 event throughput
    Web3Throughput,
    /// Benchmark Web3 event latency
    Web3Latency,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("benchmarks=info,warn")
        .init();

    let args = Args::parse();
    let client = WorkOrderClient::connect(args.node_grpc_addr.clone()).await?;

    match args.command {
        Benchmark::Idle => {
            info!("--- Running Idle Worker Capacity Benchmark ---");
            run_idle_worker_benchmark(client.clone()).await?;
        }
        Benchmark::Cpu => {
            info!("--- Running CPU-Bound Active Worker Capacity Benchmark ---");
            run_cpu_bound_worker_benchmark(client.clone()).await?;
        }
        Benchmark::Io => {
            info!("--- Running IO-Bound Active Worker Capacity Benchmark ---");
            run_io_bound_worker_benchmark(client.clone()).await?;
        }
        Benchmark::Realistic => {
            info!("--- Running Realistic Active Worker Capacity Benchmark ---");
            run_realistic_worker_benchmark(client.clone()).await?;
        }
        Benchmark::Web3Throughput => {
            info!("--- Running Web3 Event Throughput Benchmark ---");
            run_web3_throughput_benchmark(client.clone(), &args.anvil_rpc_url).await?;
        }
        Benchmark::Web3Latency => {
            info!("--- Running Web3 Event Latency Benchmark ---");
            run_web3_latency_benchmark(client.clone(), &args.anvil_rpc_url).await?;
        }
    }

    Ok(())
}

async fn run_idle_worker_benchmark(mut client: WorkOrderClient<Channel>) -> Result<()> {
    let bar = ProgressBar::new_spinner();
    bar.set_style(ProgressStyle::default_spinner().template("{spinner:.blue} {msg}")?);
    bar.set_message("Starting idle workers...");

    let mut count = 0;
    loop {
        let work_order = utils::create_work_order("cross_chain_worker.js", None, None)?;
        let request = utils::create_submit_request(work_order)?;

        match client.submit_work_order(request).await {
            Ok(response) => {
                if response.into_inner().success {
                    count += 1;
                    bar.set_message(format!("Started {} idle workers...", count));
                } else {
                    info!(
                        "Failed to start worker {}. Assuming capacity reached.",
                        count + 1
                    );
                    break;
                }
            }
            Err(e) => {
                info!(
                    "Error submitting work order {}: {}. Assuming capacity reached.",
                    count + 1,
                    e
                );
                break;
            }
        }
    }

    bar.finish_with_message(format!("Idle Worker Capacity: {}", count));
    Ok(())
}

async fn run_cpu_bound_worker_benchmark(mut client: WorkOrderClient<Channel>) -> Result<()> {
    info!("Testing CPU-bound workers...");
    let cpu_config = serde_json::json!({ "iterations": 1_000_000_000 });
    let cpu_count =
        run_active_benchmark_scenario(&mut client, "cpu_bound_worker.js", Some(cpu_config)).await?;
    info!("CPU-Bound Worker Capacity: {}", cpu_count);
    Ok(())
}

async fn run_io_bound_worker_benchmark(mut client: WorkOrderClient<Channel>) -> Result<()> {
    info!("Testing IO-bound workers...");
    let io_config = serde_json::json!({ "concurrency": 10, "delay_ms": 10 * 60 * 1000 });
    let io_count =
        run_active_benchmark_scenario(&mut client, "io_bound_worker.js", Some(io_config)).await?;
    info!("IO-Bound Worker Capacity: {}", io_count);
    Ok(())
}

async fn run_realistic_worker_benchmark(mut client: WorkOrderClient<Channel>) -> Result<()> {
    info!("Testing realistic workers...");
    let count =
        run_active_benchmark_scenario(&mut client, "realistic_worker.js", None).await?;
    info!("Realistic Worker Capacity: {}", count);
    Ok(())
}

async fn run_active_benchmark_scenario(
    client: &mut WorkOrderClient<Channel>,
    worker_path: &str,
    userdata: Option<serde_json::Value>,
) -> Result<u64> {
    let mut count = 0;
    const SUBMISSION_TIMEOUT: Duration = Duration::from_secs(5);
    const WORKER_START_DELAY: Duration = Duration::from_millis(100);

    loop {
        let launch_event = WorkerEvent {
            handler: "launch".to_string(),
            kind: WorkerEventKind::Launch,
        };
        let work_order =
            utils::create_work_order(worker_path, userdata.clone(), Some(vec![launch_event]))?;
        let request = utils::create_submit_request(work_order)?;

        // Race the submission with a timeout
        let submission_result =
            tokio::time::timeout(SUBMISSION_TIMEOUT, client.submit_work_order(request)).await;

        match submission_result {
            Ok(Ok(response)) => {
                if !response.into_inner().success {
                    info!(
                        "Failed to start worker {}. Assuming capacity reached.",
                        count + 1
                    );
                    break;
                }
            }
            Ok(Err(e)) => {
                info!(
                    "Error submitting work order {}: {}. Assuming capacity reached.",
                    count + 1,
                    e
                );
                break;
            }
            Err(_) => {
                info!(
                    "Timeout submitting work order {}. Assuming capacity reached.",
                    count + 1
                );
                break;
            }
        }

        count += 1;

        // Pause momentarily to allow the active worker to begin
        tokio::time::sleep(WORKER_START_DELAY).await;

        if count % 10 == 0 {
            info!("Started {} active workers...", count);
        }
    }
    Ok(count)
}

async fn run_web3_throughput_benchmark(
    mut client: WorkOrderClient<Channel>,
    anvil_url: &str,
) -> Result<()> {
    let (_provider, contract, contract_abi) = utils::deploy_test_events_contract(anvil_url).await?;

    let work_order = utils::create_cross_chain_work_order(anvil_url, anvil_url, &contract_abi)?;
    let request = utils::create_submit_request(work_order)?;

    match client.submit_work_order(request).await {
        Ok(response) => {
            if !response.into_inner().success {
                return Err(anyhow::anyhow!("Failed to start web3 event worker"));
            }
        }
        Err(e) => {
            return Err(anyhow::anyhow!("Error starting web3 event worker: {}", e));
        }
    }

    info!("Web3 event worker started. Waiting for it to be ready...");
    tokio::time::sleep(Duration::from_secs(3)).await;

    info!("Starting event emission...");
    let total_duration = Duration::from_secs(10);
    let start_time = Instant::now();
    let mut event_count = 0;

    while start_time.elapsed() < total_duration {
        contract
            .triggerEvent(U256::from(42), vec![].into())
            .send()
            .await?
            .get_receipt()
            .await?;
        event_count += 1;
    }

    let elapsed = start_time.elapsed();
    let throughput = event_count as f64 / elapsed.as_secs_f64();

    info!(
        "Web3 Event Throughput: {:.2} events/sec ({} events in {:?})",
        throughput, event_count, elapsed
    );
    Ok(())
}

async fn run_web3_latency_benchmark(
    mut client: WorkOrderClient<Channel>,
    anvil_url: &str,
) -> Result<()> {
    let (_provider, contract, contract_abi) = utils::deploy_test_events_contract(anvil_url).await?;

    let work_order = utils::create_cross_chain_work_order(anvil_url, anvil_url, &contract_abi)?;
    let request = utils::create_submit_request(work_order)?;

    match client.submit_work_order(request).await {
        Ok(response) => {
            if !response.into_inner().success {
                return Err(anyhow::anyhow!("Failed to start web3 event worker"));
            }
        }
        Err(e) => {
            return Err(anyhow::anyhow!("Error starting web3 event worker: {}", e));
        }
    }

    info!("Web3 event worker started. Waiting for it to be ready...");
    tokio::time::sleep(Duration::from_secs(3)).await;

    let mut histogram = hdrhistogram::Histogram::<u64>::new(3)?;
    let num_events = 100;

    info!("Measuring latency for {} events...", num_events);
    let bar = ProgressBar::new(num_events);

    for i in 0..num_events {
        let value_to_set = 1000 + i;
        let start_time = Instant::now();
        contract
            .triggerEvent(U256::from(value_to_set), vec![].into())
            .send()
            .await?
            .get_receipt()
            .await?;

        loop {
            let current_value: u64 = contract.value().call().await?.to();
            if current_value == value_to_set {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let latency = start_time.elapsed().as_millis() as u64;
        histogram.record(latency)?;
        bar.inc(1);
    }
    bar.finish();

    info!("--- Web3 Event Latency Results (ms) ---");
    info!("Mean: {:.2}", histogram.mean());
    info!("StdDev: {:.2}", histogram.stdev());
    info!("Min: {}", histogram.min());
    info!("Max: {}", histogram.max());
    info!("p50: {}", histogram.value_at_quantile(0.5));
    info!("p90: {}", histogram.value_at_quantile(0.9));
    info!("p99: {}", histogram.value_at_quantile(0.99));

    Ok(())
}
