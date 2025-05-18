pub mod proto {
    pub mod interface {
        tonic::include_proto!("interface");
    }
    pub mod daemon {
        tonic::include_proto!("daemon");
    }
    pub mod enclave {
        tonic::include_proto!("enclave");
    }
    pub mod vm {
        tonic::include_proto!("vm");
    }
}

pub mod types;
