/// For production usage, no env vars or CLI args may override these.
/// Provide them at build time or as a static fallback.
pub struct EnclaveConfig {
    pub mode: &'static str,
    pub vsock_cid: u32,
    pub vsock_port: u32,
    pub uds_path: &'static str,
}

impl EnclaveConfig {
    /// Returns a config with vsock enabled, intended for production builds.
    pub const fn production() -> Self {
        Self {
            mode: "vsock",
            vsock_cid: 3,
            vsock_port: 60051,
            uds_path: "/tmp/enclave_grpc.sock",
        }
    }

    /// Returns a config with UDS, intended for local dev.
    pub const fn dev() -> Self {
        Self {
            mode: "uds",
            vsock_cid: 3,
            vsock_port: 60051,
            uds_path: "/tmp/enclave_grpc.sock",
        }
    }
}
