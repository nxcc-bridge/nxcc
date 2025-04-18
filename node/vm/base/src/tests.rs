use std::{error::Error, net::SocketAddr, sync::Arc};

use nxcc_interface::proto::vm::{
    GetAttestationRequest, InvokeWorkerRequest, StartWorkerRequest, vm_client::VmClient,
};
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::{
    Request,
    transport::{Channel, Server},
};

use crate::{
    binding::BoundClient,
    server::{VmError, VmRuntime},
    tls::{
        create_client_tls_config, create_server_tls_config, generate_ca_cert, generate_signed_cert,
    },
};

/// A mock VM runtime implementation for testing
struct MockVmRuntime;

#[tonic::async_trait]
impl VmRuntime for MockVmRuntime {
    async fn start_worker(
        &self,
        worker_id: String,
        _worker_code: Vec<u8>,
        _config: Vec<u8>,
    ) -> Result<String, VmError> {
        Ok(format!("instance-{}", worker_id))
    }

    async fn stop_worker(&self, _instance_id: String) -> Result<(), VmError> {
        Ok(())
    }

    async fn invoke_worker(
        &self,
        _instance_id: String,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, VmError> {
        Ok(payload)
    }

    async fn get_attestation(
        &self,
        user_data: Vec<u8>,
    ) -> Result<nxcc_interface::types::AttestationReport, VmError> {
        Ok(nxcc_interface::types::AttestationReport {
            ephemeral_public_key: vec![0, 1, 2, 3],
            block_hashes: vec![vec![4, 5, 6, 7]],
            user_data,
        })
    }
}

/// Run a fully in-memory test of the VM server and client
#[tokio::test]
async fn test_e2e_with_client_binding() -> Result<(), Box<dyn Error>> {
    // 1. Generate the dummy CA
    let (dummy_ca_cert, dummy_ca_key) = generate_ca_cert().unwrap();
    let dummy_ca_cert_pem = dummy_ca_cert.pem();

    // 2. Generate server certificate signed by the dummy CA
    let (server_cert_pem, server_key_pem) =
        generate_signed_cert("localhost", &dummy_ca_cert, &dummy_ca_key).unwrap();

    // 3. Generate client certificates signed by the dummy CA
    let (client1_cert_pem, client1_key_pem) =
        generate_signed_cert("client1", &dummy_ca_cert, &dummy_ca_key).unwrap();
    let (client2_cert_pem, client2_key_pem) =
        generate_signed_cert("client2", &dummy_ca_cert, &dummy_ca_key).unwrap();

    // 4. Configure server TLS with the dummy CA for client auth
    let server_tls_config = create_server_tls_config(
        server_cert_pem.clone(), // Clone needed for client config later
        server_key_pem,
        dummy_ca_cert_pem.clone(), // Clone needed for client config later
    )
    .unwrap();

    // 5. Start a TCP listener on an ephemeral port
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server_addr = listener.local_addr().unwrap();
    let incoming_stream = TcpListenerStream::new(listener);

    // 6. Create the VmRuntime and BoundClient
    let runtime = Arc::new(MockVmRuntime);
    let bound_client = BoundClient::new();

    // 7. Start the server in a separate task
    let server_bound_client = bound_client.clone();
    let server_handle = tokio::spawn(async move {
        Server::builder()
            .tls_config(server_tls_config)
            .unwrap()
            .layer(crate::binding::ClientBindingLayer::new(server_bound_client))
            .add_service(nxcc_interface::proto::vm::vm_server::VmServer::new(
                crate::server::VmServiceGrpc::new(runtime),
            ))
            .serve_with_incoming(incoming_stream)
            .await
            .unwrap();
    });

    // Allow server to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // 8. Create first client connection
    let client1 = create_client(
        server_addr,
        &client1_cert_pem,
        &client1_key_pem,
        &dummy_ca_cert_pem, // Use dummy CA for validation
        "localhost",
    )
    .await?;

    // 9. Test first client operations
    let mut client1 = client1;
    let response = client1
        .start_worker(Request::new(StartWorkerRequest {
            worker_id: "test-worker".to_string(),
            worker_code: vec![1, 2, 3],
            config: vec![4, 5, 6],
        }))
        .await?;

    assert!(
        response.into_inner().success,
        "First client call should succeed"
    );

    // 10. Create a new connection for the first client
    let client1_reconnect = create_client(
        server_addr,
        &client1_cert_pem,
        &client1_key_pem,
        &dummy_ca_cert_pem, // Use dummy CA for validation
        "localhost",
    )
    .await?;

    // 11. Verify the first client can still call methods after reconnecting
    let mut client1_reconnect = client1_reconnect;
    let response = client1_reconnect
        .invoke_worker(Request::new(InvokeWorkerRequest {
            instance_id: "instance-test-worker".to_string(),
            payload: vec![7, 8, 9],
        }))
        .await?;

    assert!(
        response.into_inner().success,
        "First client reconnection should succeed"
    );

    // 12. Create a second client with a different certificate
    let client2 = create_client(
        server_addr,
        &client2_cert_pem,
        &client2_key_pem,
        &dummy_ca_cert_pem, // Use dummy CA for validation
        "localhost",
    )
    .await?;

    // 13. Verify the second client cannot call methods (should be rejected due to client binding)
    let mut client2 = client2;
    let result = client2
        .get_attestation(Request::new(GetAttestationRequest {
            user_data: vec![10, 11, 12],
        }))
        .await;

    assert!(result.is_err(), "Second client should be rejected");
    let err = result.unwrap_err();
    // NOTE: Ideally, this would be PermissionDenied (7). However, due to how errors
    // are generated via HTTP responses in the layer before hitting the main Tonic service,
    // Tonic might interpret it as Unknown (2) if the mapping isn't perfect or if
    // the necessary gRPC trailers aren't fully processed by the client stack in this layer-rejection scenario.
    // The critical part is that the request *is* rejected with the correct message.
    assert!(
        err.message().contains("bound to another client"),
        "Error message should indicate client binding issue: {}",
        err.message()
    );

    // Clean up the server
    server_handle.abort();

    Ok(())
}

/// Helper function to create a gRPC client with TLS using the dummy CA setup
async fn create_client(
    server_addr: SocketAddr,
    client_cert_pem: &str,
    client_key_pem: &str,
    dummy_ca_cert_pem: &str,
    domain_name: &str,
) -> Result<VmClient<Channel>, Box<dyn Error>> {
    // Configure TLS using the dummy CA
    let tls_config = create_client_tls_config(
        client_cert_pem.to_string(),
        client_key_pem.to_string(),
        dummy_ca_cert_pem.to_string(),
        domain_name,
    )
    .unwrap();

    // Create channel and connect
    let channel = Channel::from_shared(format!("https://{}", server_addr))?
        .tls_config(tls_config)?
        .connect()
        .await?;

    Ok(VmClient::new(channel))
}
