#![allow(warnings)]

mod config;
mod crypto;
mod grpc;
mod runner;
mod secrets;
#[cfg(test)]
mod tests;

use config::EnclaveConfig;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let config = match EnclaveConfig::load() {
        Ok(cfg) => cfg,
        Err(e) => {
            error!("Failed to load enclave configuration: {}", e);
            return Err(e.into());
        }
    };

    info!("Starting enclave with configuration: {:?}", config);

    grpc::start_grpc_server(&config).await?;
    Ok(())
}
