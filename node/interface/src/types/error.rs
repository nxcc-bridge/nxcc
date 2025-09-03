use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConversionError {
    #[error("Missing field in protobuf message: {0}")]
    MissingField(String),
    #[error("Invalid value for field {field}: {message}")]
    InvalidValue { field: String, message: String },
    #[error("JSON deserialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Base64 decoding error: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("Invalid DSSE payload type: expected {expected}, got {got}")]
    InvalidDssePayloadType { expected: String, got: String },
    #[error("Invalid byte slice length for {name}: expected {expected}, got {got}")]
    InvalidSliceLength {
        name: String,
        expected: usize,
        got: usize,
    },
}
