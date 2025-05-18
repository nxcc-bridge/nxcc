#![allow(warnings)]

pub mod binding;
pub mod client;
pub mod config;
pub mod server;
#[cfg(test)]
mod tests;
pub mod tls;

use std::{error::Error, sync::Arc};

use crate::{
    server::{ServerConfig, VmRuntime, run_vm_server},
    tls::MtlsCertificates,
};

/// Run a VM server with auto-generated TLS certificates signed by a dummy CA.
///
/// This function generates a new set of mTLS certificates (CA, server, client)
/// for each invocation using a self-signed dummy CA. It then configures and
/// runs the gRPC server using the generated server certificate and CA.
///
/// The generated client certificate and CA certificate are not returned by this function,
/// but can be generated independently using `MtlsCertificates::new()` if needed
/// for creating a client to connect to this server.
pub async fn run_server<T: VmRuntime>(
    config: ServerConfig,
    runtime: Arc<T>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    // Generate the full mTLS certificate set (CA, server, client)
    // We only need the server parts and CA to run the server itself.
    let certs = MtlsCertificates::new()?;

    // Create server TLS configuration using the generated certificates
    let server_tls_config = certs.server_tls_config()?;

    // Run the server
    run_vm_server(config, runtime, server_tls_config).await
}
