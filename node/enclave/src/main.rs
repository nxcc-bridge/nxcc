#![allow(warnings)]

mod attestation;
mod config;
mod crypto;
mod grpc;
mod runner;
mod secrets;
#[cfg(test)]
mod tests;

use config::EnclaveConfig;
use tracing::{Level, error, info};
use tracing_subscriber::{EnvFilter, FmtSubscriber as Subscriber, fmt::format::FmtSpan};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = EnclaveConfig::load();

    // Set up logging with the appropriate level based on verbose flag
    let log_level = if config.verbose {
        Level::DEBUG
    } else {
        Level::INFO
    };
    let base_filter = format!("{}={}", env!("CARGO_PKG_NAME").replace("-", "_"), log_level);
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(base_filter));
    let subscriber = Subscriber::builder()
        .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
        .with_env_filter(env_filter)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    info!("Starting enclave with configuration: {:?}", config);

    grpc::start_grpc_server(&config).await?;
    Ok(())
}
