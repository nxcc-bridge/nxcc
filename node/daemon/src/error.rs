use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Configuration error: {0}")]
    Config(#[from] figment::Error),

    #[error("Network error: {0}")]
    Network(String),

    #[error("Identity error: {0}")]
    Identity(String),

    #[error("Service error: {0}")]
    Service(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Authorization error: {0}")]
    Authorization(String),
}

impl From<libp2p::core::transport::TransportError<std::io::Error>> for AppError {
    fn from(err: libp2p::core::transport::TransportError<std::io::Error>) -> Self {
        Self::Network(format!("Transport error: {}", err))
    }
}

impl<T> From<futures::channel::mpsc::TrySendError<T>> for AppError {
    fn from(err: futures::channel::mpsc::TrySendError<T>) -> Self {
        Self::Network(format!("Send error: {}", err))
    }
}

impl From<libp2p::gossipsub::SubscriptionError> for AppError {
    fn from(err: libp2p::gossipsub::SubscriptionError) -> Self {
        Self::Network(format!("Gossipsub Subscription error: {}", err))
    }
}

impl From<libp2p::core::multiaddr::Error> for AppError {
    fn from(err: libp2p::core::multiaddr::Error) -> Self {
        Self::Network(format!("Multiaddr error: {}", err))
    }
}

impl From<libp2p::noise::Error> for AppError {
    fn from(err: libp2p::noise::Error) -> Self {
        Self::Network(format!("Noise error: {}", err))
    }
}
