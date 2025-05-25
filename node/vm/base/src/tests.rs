use std::{collections::HashMap, error::Error, sync::Arc};

use nxcc_interface::proto::vm::{
    Header as ProtoHeader, HttpRequest as ProtoHttpRequest, HttpResponse as ProtoHttpResponse,
    TrustedConfig, UntrustedConfig, WorkerStatus,
};
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::{Certificate, ClientTlsConfig, Identity, Server};

use crate::{
    binding::BoundClient,
    client::{ClientError, VmClient as _, VmServiceClient},
    server::{VmError, VmRuntime, VmServiceGrpc},
    tls::MtlsCertificates,
};

/// A mock VM runtime implementation for the *server side* of this E2E test.
#[derive(Default)]
struct E2EMockVmRuntime;

#[tonic::async_trait]
impl VmRuntime for E2EMockVmRuntime {
    async fn start_worker(
        &self,
        _worker_code: Vec<u8>,
        _untrusted_config: UntrustedConfig,
        _trusted_config: TrustedConfig,
    ) -> Result<String, VmError> {
        Ok(format!("instance-e2e-{}", rand::random::<u16>()))
    }

    async fn stop_worker(&self, id: String) -> Result<(), VmError> {
        if id.starts_with("instance-e2e-") {
            Ok(())
        } else {
            Err(VmError::new("Not found"))
        }
    }

    async fn invoke_worker(
        &self,
        id: String,
        _handler_name: String,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, VmError> {
        if id.starts_with("instance-e2e-") {
            Ok(payload) // Echo payload
        } else {
            Err(VmError::new("Not found"))
        }
    }

    async fn invoke_http(
        &self,
        id: String,
        request: ProtoHttpRequest,
    ) -> Result<ProtoHttpResponse, VmError> {
        if id.starts_with("instance-e2e-") {
            Ok(ProtoHttpResponse {
                status_code: 200,
                headers: request.headers,
                body: request.body,
            })
        } else {
            Err(VmError::new("Not found"))
        }
    }

    async fn get_attestation(
        &self,
        user_data: Vec<u8>,
    ) -> Result<nxcc_interface::types::AttestationReport, VmError> {
        Ok(nxcc_interface::types::AttestationReport {
            measurement: vec![0u8; 32],
            ephemeral_public_key: vec![0, 1, 2, 3],
            block_hashes: vec![vec![4, 5, 6, 7]],
            user_data,
        })
    }

    async fn get_worker_status(&self, id: String) -> Result<WorkerStatus, VmError> {
        if id.starts_with("instance-e2e-") {
            Ok(WorkerStatus::Running)
        } else {
            Err(VmError::new("Not found"))
        }
    }

    async fn list_running_workers(&self) -> Result<Vec<String>, VmError> {
        Ok(vec![
            "instance-e2e-1".to_string(),
            "instance-e2e-2".to_string(),
        ])
    }

    async fn get_worker_logs(&self, id: String) -> Result<String, VmError> {
        if id.starts_with("instance-e2e-") {
            Ok(format!("Mock logs for {}", id))
        } else {
            Err(VmError::new("Not found"))
        }
    }
}

/// Run a fully in-memory test of the VM server and client binding layer.
#[tokio::test]
async fn test_e2e_with_client_binding() -> Result<(), Box<dyn Error>> {
    // 1. Generate CA and Certs using the simplified API
    let certs = MtlsCertificates::new()?;

    // 2. Configure server TLS
    let server_tls_config = certs.server_tls_config()?;

    // 3. Start listener and server with E2EMockVmRuntime
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server_addr = listener.local_addr().unwrap();
    let incoming_stream = TcpListenerStream::new(listener);
    let runtime = Arc::new(E2EMockVmRuntime); // Use the E2E mock
    let bound_client = BoundClient::new();
    let server_bound_client = bound_client.clone(); // Clone for the server
    let server_handle = tokio::spawn(async move {
        Server::builder()
            .tls_config(server_tls_config) // Use generated config
            .unwrap()
            .layer(crate::binding::ClientBindingLayer::new(server_bound_client))
            .add_service(nxcc_interface::proto::vm::vm_server::VmServer::new(
                VmServiceGrpc::new(runtime),
            ))
            .serve_with_incoming(incoming_stream)
            .await
            .unwrap();
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await; // Allow server to start

    // 4. Create first client connection using VmServiceClient and generated config
    let client1_tls_config = certs.client_tls_config()?;
    let mut client1 = VmServiceClient::connect(server_addr, client1_tls_config.clone()).await?; // Clone config for potential reuse

    // 5. Test first client operations (StartWorker)
    let untrusted_config = UntrustedConfig {
        userdata_json: r#"{"key":"value"}"#.to_string(),
        advanced_vm_config: HashMap::new(),
    };
    let mut secrets = HashMap::new();
    secrets.insert("TEST_SECRET".to_string(), vec![10, 11, 12]);
    let trusted_config = TrustedConfig {
        secrets,
        limits: None,
    };
    let start_result = client1
        .start_worker(
            "test-worker".to_string(),
            vec![1, 2, 3],
            untrusted_config,
            trusted_config,
        )
        .await;
    assert!(start_result.is_ok(), "First client StartWorker failed");
    let worker_id = start_result.unwrap();
    assert!(worker_id.starts_with("instance-e2e-"));

    // 6. Test first client operations (GetWorkerStatus)
    let status_result = client1.get_worker_status(worker_id.clone()).await;
    assert!(status_result.is_ok(), "First client GetWorkerStatus failed");
    assert_eq!(status_result.unwrap(), WorkerStatus::Running);

    // 7. Test first client operations (ListRunningWorkers)
    let list_result = client1.list_running_workers().await;
    assert!(
        list_result.is_ok(),
        "First client ListRunningWorkers failed"
    );
    let worker_ids = list_result.unwrap();
    // Note: E2E mock returns fixed list, doesn't track started workers
    assert!(worker_ids.contains(&"instance-e2e-1".to_string()));

    // 8. Test first client operations (GetWorkerLogs)
    let logs_result = client1.get_worker_logs(worker_id.clone()).await;
    assert!(logs_result.is_ok(), "First client GetWorkerLogs failed");
    assert!(logs_result.unwrap().contains(&worker_id));

    // 9. Create a new connection for the first client (reuse TLS config)
    let mut client1_reconnect = VmServiceClient::connect(server_addr, client1_tls_config).await?;

    // 10. Verify the first client can still call methods after reconnecting (InvokeWorker)
    let invoke_result = client1_reconnect
        .invoke_worker(
            worker_id.clone(),
            "default_handler".to_string(),
            vec![7, 8, 9],
        )
        .await;
    assert!(
        invoke_result.is_ok(),
        "First client reconnection InvokeWorker failed"
    );
    assert_eq!(invoke_result.unwrap(), vec![7, 8, 9]); // Mock echoes

    // 10. Test first client operations (InvokeHttp)
    let http_request_payload = ProtoHttpRequest {
        method: "POST".to_string(),
        uri: "/e2e/resource".to_string(),
        headers: vec![ProtoHeader {
            key: "Content-Type".to_string(),
            value: b"application/json".to_vec(),
        }],
        body: b"{\"data\":\"payload\"}".to_vec(),
    };
    let http_invoke_result = client1_reconnect
        .invoke_http(worker_id.clone(), http_request_payload.clone())
        .await;
    assert!(http_invoke_result.is_ok(), "First client InvokeHttp failed");
    let http_response = http_invoke_result.unwrap();
    assert_eq!(http_response.status_code, 200);
    assert_eq!(http_response.body, http_request_payload.body);

    // 11. Create a second client with a *different* certificate signed by the *same* CA
    let client2_bundle = certs.generate_additional_client_cert("client2")?;
    let client2_identity = Identity::from_pem(&client2_bundle.cert_pem, &client2_bundle.key_pem);
    let ca_cert = Certificate::from_pem(&certs.ca_pem);
    let client2_tls_config = ClientTlsConfig::new()
        .identity(client2_identity)
        .ca_certificate(ca_cert)
        .domain_name("localhost"); // Still need domain name

    let mut client2 = VmServiceClient::connect(server_addr, client2_tls_config).await?;

    // 12. Verify the second client cannot call methods (GetAttestation) because binding layer rejects it
    let result = client2.get_attestation(vec![10, 11, 12]).await;
    assert!(result.is_err(), "Second client should be rejected");
    match result.err().unwrap() {
        ClientError::Grpc(status) => {
            assert!(
                status.message().contains("bound to another client"),
                "Error message mismatch: {}",
                status.message()
            );
        }
        e => panic!("Expected Grpc error, got {:?}", e),
    }

    // 13. Verify the second client cannot call other methods (GetWorkerStatus)
    let result_status = client2.get_worker_status(worker_id.clone()).await;
    assert!(
        result_status.is_err(),
        "Second client GetWorkerStatus should be rejected"
    );
    result_status.err().unwrap();

    // Clean up the server
    server_handle.abort();

    Ok(())
}
