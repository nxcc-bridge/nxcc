use std::path::PathBuf;

use nxcc_vm_base::server::VmError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum WorkerdVmError {
    #[error("Worker instance not found: {0}")]
    WorkerNotFound(String),

    #[error("Worker process failed to start: {0}")]
    ProcessStartFailed(std::io::Error),

    #[error("Worker process terminated unexpectedly with status: {0:?}")]
    ProcessTerminated(Option<std::process::ExitStatus>),

    #[error("Failed to generate Cap'n Proto config: {0}")]
    ConfigGenerationFailed(String),

    #[error("Failed to serialize Cap'n Proto config: {0}")]
    ConfigSerializationFailed(#[from] capnp::Error),

    #[error("Failed to write config file: {0}")]
    ConfigFileWriteFailed(std::io::Error),

    #[error("Failed to create temporary directory: {0}")]
    TempDirCreationFailed(std::io::Error),

    #[error("Failed to determine workerd binary path")]
    WorkerdBinaryNotFound, // TODO: Make this configurable or discoverable

    #[error("Unsupported worker code type")]
    UnsupportedCodeType,

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("Failed to communicate with worker via UDS {path}: {source}")]
    UdsCommunicationFailed {
        path: PathBuf,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("Worker invocation failed with status {status}: {body}")]
    InvocationHttpFailed { status: u16, body: String },

    #[error("Worker returned invalid response: {0}")]
    InvalidWorkerResponse(String),

    #[error("Failed to parse JSON config: {0}")]
    JsonConfigParseFailed(#[from] serde_json::Error),

    #[error("Failed to parse secret key (JWK): {0}")]
    SecretKeyParseFailed(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Attestation not supported by workerd VMM")]
    AttestationNotSupported,

    #[error("Worker is not in a runnable state: {0:?}")]
    WorkerNotRunnable(nxcc_interface::proto::vm::WorkerStatus),
}

impl From<WorkerdVmError> for VmError {
    fn from(e: WorkerdVmError) -> Self {
        // Simple conversion for now, could add more context mapping later
        VmError::new(e.to_string())
    }
}

// Helper to box hyper errors for UdsCommunicationFailed
impl From<hyper::Error> for WorkerdVmError {
    fn from(e: hyper::Error) -> Self {
        WorkerdVmError::UdsCommunicationFailed {
            path: PathBuf::from("unknown"), // Path should be added contextually
            source: Box::new(e),
        }
    }
}
