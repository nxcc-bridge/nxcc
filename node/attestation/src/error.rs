use thiserror::Error;

#[derive(Error, Debug)]
pub enum AttestationError {
    #[error("Platform not supported: {0}")]
    UnsupportedPlatform(String),

    #[error("No providers available for platform: {0}")]
    NoProvidersAvailable(String),

    #[error("All providers failed for platform: {0}")]
    AllProvidersFailed(String),

    #[error("Invalid attestation format: {0}")]
    InvalidFormat(String),

    #[error("Verification failed: {0}")]
    VerificationFailed(String),

    #[error("Configuration error: {0}")]
    ConfigurationError(String),

    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Generic error: {0}")]
    Other(#[from] anyhow::Error),
}
