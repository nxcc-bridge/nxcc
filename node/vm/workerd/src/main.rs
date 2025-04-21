mod config_builder;
mod errors;
mod vmm;

#[allow(warnings)]
pub mod workerd_capnp {
    include!(concat!(env!("OUT_DIR"), "/workerd_capnp.rs"));
}

use std::{error::Error, sync::Arc};

use nxcc_vm_base::{run_server, server::ServerConfig};
use tracing::Level;
use tracing_subscriber::FmtSubscriber;
use vmm::WorkerdVmm;

// TODO: Use clap or similar for proper argument parsing
fn get_server_config() -> ServerConfig {
    // Default to UDS for now, make configurable via args later
    let uds_path = "/tmp/nxcc-workerd-vmm.sock".to_string();
    println!("Defaulting to UDS server at: {}", uds_path);
    ServerConfig::Uds { path: uds_path }
    // Example TCP:
    // ServerConfig::Tcp { addr: "127.0.0.1:50051".parse().unwrap() }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    // Initialize tracing subscriber
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::DEBUG) // Adjust log level as needed
        .with_env_filter("nxcc_workerd_vm=debug,nxcc_vm_base=debug,info") // Example filter
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("Setting default subscriber failed");

    tracing::info!("Starting Workerd VMM...");

    let config = get_server_config();
    let runtime = Arc::new(WorkerdVmm::new());

    run_server(config, runtime).await?;

    tracing::info!("Workerd VMM finished.");
    Ok(())
}
