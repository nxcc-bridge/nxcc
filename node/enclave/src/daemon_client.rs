use interface::proto::daemon::{
    BroadcastRequest, BroadcastResponse, daemon_client::DaemonClient as GrpcDaemonClient,
};
use tonic::transport::{Channel, Endpoint};

pub struct DaemonClient {
    inner: GrpcDaemonClient<Channel>,
}

impl DaemonClient {
    pub async fn connect_uds(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let channel = Endpoint::from_shared(format!("http://[unix://{}]", path))?
            .connect()
            .await?;
        Ok(Self {
            inner: GrpcDaemonClient::new(channel),
        })
    }

    pub async fn connect_vsock(cid: u32, port: u32) -> Result<Self, Box<dyn std::error::Error>> {
        // For real vsock usage, you’d need a custom connector, e.g. wrapping a VsockStream.
        // This snippet is just a placeholder.
        todo!("Implement vsock-based DaemonClient connection if required");
    }

    pub async fn broadcast_notification(
        &mut self,
        msg: &str,
    ) -> Result<BroadcastResponse, tonic::Status> {
        let req = BroadcastRequest {
            message: msg.to_string(),
        };
        let resp = self.inner.broadcast_notification(req).await?;
        Ok(resp.into_inner())
    }
}
