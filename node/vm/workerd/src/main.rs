#![allow(warnings)]

mod config;
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let config = config::Config::load().expect("Config load failed");

    // Initialize tracing subscriber
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::DEBUG) // Adjust log level as needed
        .with_env_filter("nxcc_workerd_vm=debug,nxcc_vm_base=debug,info") // Example filter
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("Setting default subscriber failed");

    tracing::info!("Starting Workerd VMM with configuration: {:?}", config);

    let server_config: ServerConfig = (&config.base.server)
        .try_into()
        .expect("Failed to create server configuration");
    let runtime = Arc::new(WorkerdVmm::new(config.workerd.clone()));

    run_server(server_config, runtime).await?;

    tracing::info!("Workerd VMM finished.");
    Ok(())
}
