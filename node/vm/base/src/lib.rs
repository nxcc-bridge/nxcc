pub mod binding;
pub mod client;
pub mod server;
#[cfg(test)]
mod tests;
pub mod tls;

use std::{error::Error, sync::Arc};

use crate::{
    server::{ServerConfig, VmRuntime, run_vm_server},
    tls::{create_server_tls_config, generate_ca_cert, generate_signed_cert},
};

/// Run a VM server with auto-generated TLS certificates signed by a dummy CA.
pub async fn run_server<T: VmRuntime>(
    config: ServerConfig,
    runtime: Arc<T>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    // Generate the dummy CA certificate and key pair
    let (dummy_ca_cert, dummy_ca_key) = generate_ca_cert()?;
    let dummy_ca_cert_pem = dummy_ca_cert.pem();

    // Generate server's certificate signed by the dummy CA
    let (server_cert_pem, server_key_pem) =
        generate_signed_cert("localhost", &dummy_ca_cert, &dummy_ca_key)?;

    // Create server TLS configuration using the dummy CA
    let server_tls_config =
        create_server_tls_config(server_cert_pem, server_key_pem, dummy_ca_cert_pem)?;

    // Run the server
    run_vm_server(config, runtime, server_tls_config).await
}
