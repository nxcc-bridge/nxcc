use std::{collections::HashMap, error::Error, net::SocketAddr, sync::Arc};

use nxcc_interface::proto::vm::{
    GetAttestationRequest, GetWorkerLogsRequest, GetWorkerStatusRequest, InvokeWorkerRequest,
    ListRunningWorkersRequest, StartWorkerRequest, TrustedConfig, UntrustedConfig, WorkerStatus,
    vm_client::VmClient,
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
#[derive(Default)]
struct MockVmRuntime;

#[tonic::async_trait]
impl VmRuntime for MockVmRuntime {
    async fn start_worker(
        &self,
        worker_id: String,
        _worker_code: Vec<u8>,
        _untrusted_config: UntrustedConfig,
        _trusted_config: TrustedConfig,
    ) -> Result<String, VmError> {
        Ok(format!("instance-{}", worker_id))
    }

    async fn stop_worker(&self, id: String) -> Result<(), VmError> {
        if id.starts_with("instance-") {
            Ok(())
        } else {
            Err(VmError::new("Not found"))
        }
    }

    async fn invoke_worker(&self, id: String, payload: Vec<u8>) -> Result<Vec<u8>, VmError> {
        if id.starts_with("instance-") {
            Ok(payload) // Echo payload
        } else {
            Err(VmError::new("Not found"))
        }
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

    async fn get_worker_status(&self, id: String) -> Result<WorkerStatus, VmError> {
        if id.starts_with("instance-") {
            Ok(WorkerStatus::Running)
        } else {
            Err(VmError::new("Not found"))
        }
    }

    async fn list_running_workers(&self) -> Result<Vec<String>, VmError> {
        // Return a fixed list for simplicity in this mock
        Ok(vec![
            "instance-test-worker".to_string(),
            "instance-other".to_string(),
        ])
    }

    async fn get_worker_logs(&self, id: String) -> Result<String, VmError> {
        if id.starts_with("instance-") {
            Ok(format!("Mock logs for {}", id))
        } else {
            Err(VmError::new("Not found"))
        }
    }
}

/// Run a fully in-memory test of the VM server and client
#[tokio::test]
async fn test_e2e_with_client_binding() -> Result<(), Box<dyn Error>> {
    // 1. Generate CA and Certs
    let (dummy_ca_cert, dummy_ca_key) = generate_ca_cert().unwrap();
    let dummy_ca_cert_pem = dummy_ca_cert.pem();
    let (server_cert_pem, server_key_pem) =
        generate_signed_cert("localhost", &dummy_ca_cert, &dummy_ca_key).unwrap();
    let (client1_cert_pem, client1_key_pem) =
        generate_signed_cert("client1", &dummy_ca_cert, &dummy_ca_key).unwrap();
    let (client2_cert_pem, client2_key_pem) =
        generate_signed_cert("client2", &dummy_ca_cert, &dummy_ca_key).unwrap();

    // 2. Configure server TLS
    let server_tls_config = create_server_tls_config(
        server_cert_pem.clone(),
        server_key_pem,
        dummy_ca_cert_pem.clone(),
    )
    .unwrap();

    // 3. Start listener and server
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server_addr = listener.local_addr().unwrap();
    let incoming_stream = TcpListenerStream::new(listener);
    let runtime = Arc::new(MockVmRuntime);
    let bound_client = BoundClient::new();
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
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // 4. Create first client connection
    let client1 = create_client(
        server_addr,
        &client1_cert_pem,
        &client1_key_pem,
        &dummy_ca_cert_pem,
        "localhost",
    )
    .await?;

    // 5. Test first client operations (StartWorker)
    let mut client1 = client1;
    let start_response = client1
        .start_worker(Request::new(StartWorkerRequest {
            worker_id: "test-worker".to_string(),
            worker_code: vec![1, 2, 3],
            untrusted_config: Some(UntrustedConfig {
                userdata_json: r#"{"key":"value"}"#.to_string(),
                advanced_vm_config: HashMap::new(),
            }),
            trusted_config: Some(TrustedConfig {
                crypto_keys: vec![vec![10, 11, 12]], // Example serialized JWK
                limits: None,
            }),
        }))
        .await?;
    let start_response_inner = start_response.into_inner();
    assert!(
        start_response_inner.success,
        "First client StartWorker call should succeed"
    );
    let worker_id = start_response_inner.id;
    assert_eq!(worker_id, "instance-test-worker");

    // 6. Test first client operations (GetWorkerStatus)
    let status_response = client1
        .get_worker_status(Request::new(GetWorkerStatusRequest {
            id: worker_id.clone(),
        }))
        .await?;
    let status_response_inner = status_response.into_inner();
    assert!(
        status_response_inner.success,
        "First client GetWorkerStatus call should succeed"
    );
    assert_eq!(
        WorkerStatus::try_from(status_response_inner.status).unwrap(),
        WorkerStatus::Running
    );

    // 7. Test first client operations (ListRunningWorkers)
    let list_response = client1
        .list_running_workers(Request::new(ListRunningWorkersRequest {}))
        .await?;
    let list_response_inner = list_response.into_inner();
    assert!(list_response_inner.ids.contains(&worker_id));

    // 8. Test first client operations (GetWorkerLogs)
    let logs_response = client1
        .get_worker_logs(Request::new(GetWorkerLogsRequest {
            id: worker_id.clone(),
        }))
        .await?;
    let logs_response_inner = logs_response.into_inner();
    assert!(
        logs_response_inner.success,
        "First client GetWorkerLogs call should succeed"
    );
    assert!(logs_response_inner.logs.contains(&worker_id));

    // 9. Create a new connection for the first client
    let client1_reconnect = create_client(
        server_addr,
        &client1_cert_pem,
        &client1_key_pem,
        &dummy_ca_cert_pem,
        "localhost",
    )
    .await?;

    // 10. Verify the first client can still call methods after reconnecting (InvokeWorker)
    let mut client1_reconnect = client1_reconnect;
    let invoke_response = client1_reconnect
        .invoke_worker(Request::new(InvokeWorkerRequest {
            id: worker_id.clone(),
            payload: vec![7, 8, 9],
        }))
        .await?;
    assert!(
        invoke_response.into_inner().success,
        "First client reconnection InvokeWorker should succeed"
    );

    // 11. Create a second client with a different certificate
    let client2 = create_client(
        server_addr,
        &client2_cert_pem,
        &client2_key_pem,
        &dummy_ca_cert_pem,
        "localhost",
    )
    .await?;

    // 12. Verify the second client cannot call methods (GetAttestation)
    let mut client2 = client2;
    let result = client2
        .get_attestation(Request::new(GetAttestationRequest {
            user_data: vec![10, 11, 12],
        }))
        .await;
    assert!(result.is_err(), "Second client should be rejected");
    let err = result.unwrap_err();
    assert!(
        err.message().contains("bound to another client"),
        "Error message should indicate client binding issue: {}",
        err.message()
    );

    // 13. Verify the second client cannot call new methods (GetWorkerStatus)
    let result_status = client2
        .get_worker_status(Request::new(GetWorkerStatusRequest {
            id: worker_id.clone(),
        }))
        .await;
    assert!(
        result_status.is_err(),
        "Second client GetWorkerStatus should be rejected"
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
    let tls_config = create_client_tls_config(
        client_cert_pem.to_string(),
        client_key_pem.to_string(),
        dummy_ca_cert_pem.to_string(),
        domain_name,
    )
    .unwrap();

    let channel = Channel::from_shared(format!("https://{}", server_addr))?
        .tls_config(tls_config)?
        .connect()
        .await?;

    Ok(VmClient::new(channel))
}
