use nxcc_vm_base::client::ClientError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RunnerError {
    #[error("VM with ID '{0}' not attached")]
    VmNotAttached(String),
    #[error("Worker with ID '{0}' not found")]
    WorkerNotFound(String),
    #[error("VM connection error: {0}")]
    VmConnection(#[from] ClientError),
    #[error("Deserialization error: {0}")]
    Deserialization(String),
    #[error("Policy execution failed in VM: {0}")]
    PolicyExecutionFailed(String),
    #[error("Internal error: {0}")]
    Internal(String),
    #[error("TLS configuration error: {0}")]
    TlsConfig(#[from] nxcc_vm_base::tls::TlsError),
    #[error("Failed to start worker in VM: {0}")]
    WorkerStartFailed(String),
    #[error("Unsupported VM address type: {0}")]
    UnsupportedVmAddress(String),
    #[error("Event delivery channel send error: {0}")]
    EventSendError(String),
}
