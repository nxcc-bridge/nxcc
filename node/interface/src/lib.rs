pub mod proto {
    pub mod daemon {
        tonic::include_proto!("daemon");
    }

    pub mod enclave {
        tonic::include_proto!("enclave");
    }
}
