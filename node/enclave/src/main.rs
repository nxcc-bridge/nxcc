mod config;
mod crypto;
mod daemon_client;
mod server;
mod services;

use config::EnclaveConfig;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    // Choose config for dev or production
    let config = EnclaveConfig::dev();
    info!("Starting enclave in mode={}", config.mode);

    server::start_grpc_server(&config).await?;
    Ok(())
}
