use nxcc_interface::proto::vm::WorkerStatus;
use nxcc_vm_base::server::VmError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ZenroomVmError {
    #[error("Worker not found: {0}")]
    WorkerNotFound(String),
    #[error("Worker not runnable: {0:?}")]
    WorkerNotRunnable(WorkerStatus),
    #[error("Attestation not supported")]
    AttestationNotSupported,
}

impl From<ZenroomVmError> for VmError {
    fn from(err: ZenroomVmError) -> Self {
        VmError::new(err.to_string())
    }
}
