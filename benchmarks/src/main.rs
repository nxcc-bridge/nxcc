use std::time::{Duration, Instant};

use alloy_primitives::U256;
use anyhow::Result;
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use nxcc_interface::{
    proto::daemon::{work_order_client::WorkOrderClient, CheckWorkerStatusRequest},
    types::{WorkerEvent, WorkerEventKind},
};
use tonic::transport::Channel;

mod utils;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long, default_value = "http://localhost:50051")]
    node_grpc_addr: String,
    #[arg(long, default_value = "http://localhost:8545")]
    anvil_rpc_url: String,
    #[arg(long, default_value = "http://anvil:8545")]
    worker_anvil_rpc_url: String,
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
            println!("--- Running Idle Worker Capacity Benchmark ---");
            run_idle_worker_benchmark(client.clone()).await?;
        }
        Benchmark::Cpu => {
            println!("--- Running CPU-Bound Active Worker Capacity Benchmark ---");
            run_cpu_bound_worker_benchmark(client.clone()).await?;
        }
        Benchmark::Io => {
            println!("--- Running IO-Bound Active Worker Capacity Benchmark ---");
            run_io_bound_worker_benchmark(client.clone()).await?;
        }
        Benchmark::Realistic => {
            println!("--- Running Realistic Active Worker Capacity Benchmark ---");
            run_realistic_worker_benchmark(client.clone()).await?;
        }
        Benchmark::Web3Throughput => {
            println!("--- Running Web3 Event Throughput Benchmark ---");
            run_web3_throughput_benchmark(
                client.clone(),
                &args.anvil_rpc_url,
                &args.worker_anvil_rpc_url,
            )
            .await?;
        }
        Benchmark::Web3Latency => {
            println!("--- Running Web3 Event Latency Benchmark ---");
            run_web3_latency_benchmark(
                client.clone(),
                &args.anvil_rpc_url,
                &args.worker_anvil_rpc_url,
            )
            .await?;
        }
    }

    Ok(())
}

async fn run_idle_worker_benchmark(mut client: WorkOrderClient<Channel>) -> Result<()> {
    let bar = ProgressBar::new_spinner();
    bar.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg} [{elapsed_precise}]")?
            .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ "),
    );
    bar.set_message("Starting idle workers...");

    let mut count = 0;
    let mut work_order_ids = Vec::new();
    const LIVENESS_CHECK_INTERVAL: usize = 10;

    loop {
        let work_order = utils::create_work_order("cross_chain_worker.js", None, None)?;
        let request = utils::create_submit_request(work_order)?;

        match client.submit_work_order(request).await {
            Ok(response) => {
                let response = response.into_inner();
                if response.success {
                    count += 1;
                    work_order_ids.push(response.work_order_id);
                    bar.set_message(format!("Started {} idle workers", count));

                    // Periodic liveness check
                    if count % LIVENESS_CHECK_INTERVAL == 0 {
                        bar.set_message(format!("Checking liveness of {} workers...", count));
                        let mut failed_count = 0;
                        for (i, work_order_id) in work_order_ids.iter().enumerate() {
                            let request = tonic::Request::new(CheckWorkerStatusRequest {
                                work_order_id: work_order_id.clone(),
                            });
                            match client.check_worker_status(request).await {
                                Ok(response) => {
                                    let status = response.into_inner();
                                    if !status.is_running {
                                        failed_count += 1;
                                        if failed_count == 1 {
                                            bar.finish_with_message(format!(
                                                "Worker {} ({}) is not running. Capacity reached.",
                                                i + 1,
                                                work_order_id
                                            ));
                                            println!("Idle Worker Capacity: {}", count - 1);
                                            return Ok(());
                                        }
                                    }
                                }
                                Err(_) => {
                                    failed_count += 1;
                                    if failed_count >= 2 {
                                        bar.finish_with_message(format!(
                                            "Multiple workers failing liveness check. Capacity \
                                             reached."
                                        ));
                                        println!("Idle Worker Capacity: {}", count - 1);
                                        return Ok(());
                                    }
                                }
                            }
                        }
                    }
                } else {
                    bar.finish_with_message(format!(
                        "Failed to start worker {}. Capacity reached.",
                        count + 1
                    ));
                    break;
                }
            }
            Err(e) => {
                bar.finish_with_message(format!(
                    "Error submitting work order {}: {}. Capacity reached.",
                    count + 1,
                    e
                ));
                break;
            }
        }
    }

    println!("Idle Worker Capacity: {}", count);
    Ok(())
}

async fn run_cpu_bound_worker_benchmark(mut client: WorkOrderClient<Channel>) -> Result<()> {
    let bar = ProgressBar::new_spinner();
    bar.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.blue} {msg} [{elapsed_precise}]")?
            .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ "),
    );
    bar.set_message("Testing CPU-bound workers...");

    let cpu_config = serde_json::json!({ "iterations": 1_000_000_000 });
    let cpu_count =
        run_active_benchmark_scenario(&mut client, "cpu_bound_worker.js", Some(cpu_config), bar)
            .await?;
    println!("CPU-Bound Worker Capacity: {}", cpu_count);
    Ok(())
}

async fn run_io_bound_worker_benchmark(mut client: WorkOrderClient<Channel>) -> Result<()> {
    let bar = ProgressBar::new_spinner();
    bar.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.blue} {msg} [{elapsed_precise}]")?
            .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ "),
    );
    bar.set_message("Testing IO-bound workers...");

    let io_config = serde_json::json!({ "concurrency": 10, "delay_ms": 10 * 60 * 1000 });
    let io_count =
        run_active_benchmark_scenario(&mut client, "io_bound_worker.js", Some(io_config), bar)
            .await?;
    println!("IO-Bound Worker Capacity: {}", io_count);
    Ok(())
}

async fn run_realistic_worker_benchmark(mut client: WorkOrderClient<Channel>) -> Result<()> {
    let bar = ProgressBar::new_spinner();
    bar.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.blue} {msg} [{elapsed_precise}]")?
            .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ "),
    );
    bar.set_message("Testing realistic workers...");

    let count =
        run_active_benchmark_scenario(&mut client, "realistic_worker.js", None, bar).await?;
    println!("Realistic Worker Capacity: {}", count);
    Ok(())
}

async fn run_active_benchmark_scenario(
    client: &mut WorkOrderClient<Channel>,
    worker_path: &str,
    userdata: Option<serde_json::Value>,
    bar: ProgressBar,
) -> Result<u64> {
    let mut count = 0;
    let mut work_order_ids = Vec::new();
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

        let submission_result =
            tokio::time::timeout(SUBMISSION_TIMEOUT, client.submit_work_order(request)).await;

        match submission_result {
            Ok(Ok(response)) => {
                let response = response.into_inner();
                if !response.success {
                    bar.finish_with_message(format!(
                        "Failed to start worker {}. Capacity reached.",
                        count + 1
                    ));
                    break;
                }
                work_order_ids.push(response.work_order_id);
            }
            Ok(Err(e)) => {
                bar.finish_with_message(format!(
                    "Error submitting work order {}: {}. Capacity reached.",
                    count + 1,
                    e
                ));
                break;
            }
            Err(_) => {
                bar.finish_with_message(format!(
                    "Timeout submitting work order {}. Capacity reached.",
                    count + 1
                ));
                break;
            }
        }

        count += 1;
        bar.set_message(format!("Started {} workers", count));

        tokio::time::sleep(WORKER_START_DELAY).await;

        if count > 0 && count % 3 == 0 {
            bar.set_message(format!("Checking status of {} workers...", count));
            for (i, work_order_id) in work_order_ids.iter().enumerate() {
                let request = tonic::Request::new(CheckWorkerStatusRequest {
                    work_order_id: work_order_id.clone(),
                });
                match client.check_worker_status(request).await {
                    Ok(response) => {
                        let status = response.into_inner();
                        if !status.is_running {
                            bar.finish_with_message(format!(
                                "Worker {} ({}) is not running (status: '{}'). Capacity reached.",
                                i + 1,
                                work_order_id,
                                status.status_message
                            ));
                            return Ok(count - 1);
                        }
                    }
                    Err(e) => {
                        bar.finish_with_message(format!(
                            "Error checking status for worker {} ({}): {}. Capacity reached.",
                            i + 1,
                            work_order_id,
                            e
                        ));
                        return Ok(count - 1);
                    }
                }
            }
        }
    }
    Ok(count)
}

async fn run_web3_throughput_benchmark(
    mut client: WorkOrderClient<Channel>,
    anvil_url: &str,
    worker_anvil_url: &str,
) -> Result<()> {
    let bar = ProgressBar::new_spinner();
    bar.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg} [{elapsed_precise}]")?
            .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ "),
    );
    bar.set_message("Deploying test contract...");

    let (_provider, contract, contract_abi) = utils::deploy_test_events_contract(anvil_url).await?;

    bar.set_message("Starting web3 event worker...");
    let work_order = utils::create_cross_chain_work_order(
        worker_anvil_url,
        worker_anvil_url,
        &contract_abi,
        contract.address(),
    )?;
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

    bar.set_message("Waiting for worker to be ready...");
    tokio::time::sleep(Duration::from_secs(3)).await;

    bar.set_message("Emitting events...");
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
        bar.set_message(format!("Emitted {} events", event_count));
        // TODO: make this actually measure throughput
    }

    let elapsed = start_time.elapsed();
    let throughput = event_count as f64 / elapsed.as_secs_f64();

    bar.finish_with_message(format!(
        "Completed: {:.2} events/sec ({} events in {:?})",
        throughput, event_count, elapsed
    ));

    println!("Web3 Event Throughput: {:.2} events/sec", throughput);
    Ok(())
}

async fn run_web3_latency_benchmark(
    mut client: WorkOrderClient<Channel>,
    anvil_url: &str,
    worker_anvil_url: &str,
) -> Result<()> {
    let setup_bar = ProgressBar::new_spinner();
    setup_bar.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")?
            .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ "),
    );
    setup_bar.set_message("Deploying test contract...");

    let (_provider, contract, contract_abi) = utils::deploy_test_events_contract(anvil_url).await?;

    setup_bar.set_message("Starting web3 event worker...");
    let work_order = utils::create_cross_chain_work_order(
        worker_anvil_url,
        worker_anvil_url,
        &contract_abi,
        contract.address(),
    )?;
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

    setup_bar.set_message("Waiting for worker to be ready...");
    tokio::time::sleep(Duration::from_secs(3)).await;
    setup_bar.finish_and_clear();

    let mut histogram = hdrhistogram::Histogram::<u64>::new(3)?;
    let num_events = 10000;

    let bar = ProgressBar::new(num_events);
    bar.set_style(
        ProgressStyle::default_bar()
            .template(
                "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg}",
            )?
            .progress_chars("#>-"),
    );
    bar.set_message("Measuring event latency");

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
        bar.set_message(format!("Latest: {}ms", latency));
    }
    bar.finish_with_message("Latency measurement complete");

    println!("\n--- Web3 Event Latency Results (ms) ---");
    println!("Mean: {:.2}", histogram.mean());
    println!("StdDev: {:.2}", histogram.stdev());
    println!("Min: {}", histogram.min());
    println!("Max: {}", histogram.max());
    println!("p50: {}", histogram.value_at_quantile(0.5));
    println!("p90: {}", histogram.value_at_quantile(0.9));
    println!("p99: {}", histogram.value_at_quantile(0.99));

    Ok(())
}
