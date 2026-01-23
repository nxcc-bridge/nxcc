mod config;
mod errors;
mod vmm;

use std::{error::Error, sync::Arc};

use nxcc_vm_base::{run_server, server::ServerConfig};
use tracing::Level;
use tracing_subscriber::{EnvFilter, FmtSubscriber as Subscriber, fmt::format::FmtSpan};

use crate::vmm::ZenroomVmm;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let config = config::Config::load();

    let log_level = if config.base.verbose {
        Level::DEBUG
    } else {
        Level::INFO
    };
    let base_filter = format!("nxcc_zenroom_vm={0},nxcc_vm_base={0}", log_level);
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(base_filter));
    let subscriber = Subscriber::builder()
        .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
        .with_env_filter(env_filter)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("Setting default subscriber failed");

    tracing::info!("Starting Zenroom VMM with configuration: {:?}", config);

    let server_config: ServerConfig = (&config.base.server)
        .try_into()
        .expect("Failed to create server configuration");
    let runtime = Arc::new(ZenroomVmm::new(config.zenroom.clone()));

    run_server(server_config, runtime).await?;

    tracing::info!("Zenroom VMM finished.");
    Ok(())
}
