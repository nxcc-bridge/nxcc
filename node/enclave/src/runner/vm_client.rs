use nxcc_interface::proto::vm::{
    HttpRequest as ProtoHttpRequest, HttpResponse as ProtoHttpResponse, TrustedConfig,
    UntrustedConfig,
};
#[cfg(test)]
use nxcc_vm_base::client::mock::MockVmServiceClient;
use nxcc_vm_base::client::{ClientError, VmClient as _, VmServiceClient};

// Define an enum to hold different VM client implementations
// Make it cfg-gated so the mock variant only exists in test builds
/// Enum for different VM client implementations
/// The mock variant is only available during tests
#[derive(Clone)]
pub enum VmClient {
    // Real VM service client
    Real(VmServiceClient),

    // Mock client only exists during tests
    #[cfg(test)]
    Mock(MockVmServiceClient),
}

// Implement the necessary methods to delegate to the inner client
impl VmClient {
    // Async function to start a worker
    pub async fn start_worker(
        &mut self,
        worker_id: String,
        worker_code: Vec<u8>,
        untrusted_config: UntrustedConfig,
        trusted_config: TrustedConfig,
    ) -> Result<String, ClientError> {
        match self {
            VmClient::Real(client) => {
                client
                    .start_worker(worker_id, worker_code, untrusted_config, trusted_config)
                    .await
            }
            #[cfg(test)]
            VmClient::Mock(client) => {
                client
                    .start_worker(worker_id, worker_code, untrusted_config, trusted_config)
                    .await
            }
        }
    }

    // Async function to stop a worker
    pub async fn stop_worker(&mut self, worker_id: String) -> Result<(), ClientError> {
        match self {
            VmClient::Real(client) => client.stop_worker(worker_id).await,
            #[cfg(test)]
            VmClient::Mock(client) => client.stop_worker(worker_id).await,
        }
    }

    // Async function to invoke a worker
    #[tracing::instrument(level = "info", skip(self, payload), fields(payload_size = payload.len()))]
    pub async fn invoke_worker(
        &mut self,
        worker_id: String,
        handler_name: String,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, ClientError> {
        match self {
            VmClient::Real(client) => client.invoke_worker(worker_id, handler_name, payload).await,
            #[cfg(test)]
            VmClient::Mock(client) => client.invoke_worker(worker_id, handler_name, payload).await,
        }
    }

    // Async function to invoke an HTTP request on a worker
    #[tracing::instrument(level = "info", skip(self, request))]
    pub async fn invoke_http(
        &mut self,
        worker_id: String,
        request: ProtoHttpRequest,
    ) -> Result<ProtoHttpResponse, ClientError> {
        match self {
            VmClient::Real(client) => client.invoke_http(worker_id, request).await,
            #[cfg(test)]
            VmClient::Mock(client) => client.invoke_http(worker_id, request).await,
        }
    }

    pub async fn probe_worker(
        &mut self,
        worker_id: String,
    ) -> Result<(nxcc_interface::proto::vm::WorkerStatus, String), ClientError> {
        match self {
            VmClient::Real(client) => client.probe_worker(worker_id).await,
            #[cfg(test)]
            VmClient::Mock(client) => client.probe_worker(worker_id).await,
        }
    }

    pub async fn get_worker_logs(&mut self, worker_id: String) -> Result<String, ClientError> {
        match self {
            VmClient::Real(client) => client.get_worker_logs(worker_id).await,
            #[cfg(test)]
            VmClient::Mock(_client) => {
                // For testing, return a simple log string
                Ok(format!("Mock log for worker {}", worker_id))
            }
        }
    }

    pub async fn stream_worker_logs(
        &mut self,
        worker_id: String,
        tail_lines: u32,
        follow: bool,
    ) -> Result<
        tokio_stream::wrappers::ReceiverStream<
            Result<nxcc_interface::proto::vm::StreamWorkerLogsResponse, tonic::Status>,
        >,
        ClientError,
    > {
        match self {
            VmClient::Real(client) => {
                client
                    .stream_worker_logs(worker_id, tail_lines, follow)
                    .await
            }
            #[cfg(test)]
            VmClient::Mock(_client) => {
                // For testing, return an empty stream
                let (tx, rx) = tokio::sync::mpsc::channel(1);
                drop(tx); // Close channel immediately
                Ok(tokio_stream::wrappers::ReceiverStream::new(rx))
            }
        }
    }
}

// Create a convenient From implementation for VmServiceClient to make client creation more ergonomic
impl From<VmServiceClient> for VmClient {
    fn from(client: VmServiceClient) -> Self {
        VmClient::Real(client)
    }
}

// Create a convenient From implementation for MockVmServiceClient when in test mode
#[cfg(test)]
impl From<MockVmServiceClient> for VmClient {
    fn from(client: MockVmServiceClient) -> Self {
        VmClient::Mock(client)
    }
}
