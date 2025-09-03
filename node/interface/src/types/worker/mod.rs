use std::collections::HashMap;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use self::events::WorkerEvent;
use super::{error::ConversionError, secrets::SecretId};

pub mod events;

/// Describes how to locate a `WorkerBundle`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerBundlePointer {
    /// The location of the `WorkerBundle`. May be a data URL for direct embedding
    /// or other schemes like http, ipfs, etc.
    pub source: url::Url,
    /// The expected SHA-512 hash of the `WorkerBundle`'s COSE envelope.
    /// Useful for mutable source URLs or content integrity checks.
    pub hash: Option<Vec<u8>>,
}

/// Describes a worker (or policy) and its inputs.
/// This is what is pointed to by the on-chain root of trust where policies are concerned.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkerManifest {
    /// An authenticated pointer to a `WorkerBundle`.
    pub bundle: WorkerBundlePointer,
    /// The set of identities that the worker needs for execution.
    /// These will be bound by the VM into the worker.
    /// Policy workers are not allowed to request identities.
    pub identities: Vec<(SecretId, String)>,
    /// Arbitrary data passed by the creator of the worker manifest.
    /// Untrusted from the perspective of the nXCC system.
    pub userdata: HashMap<String, Value>,
}

/// Represents a signature entry in a DSSE envelope.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct DsseSignatureEntry {
    #[serde(rename = "keyid", skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    pub sig: String, // base64 encoded
}

/// Represents a DSSE (Dead Simple Signing Envelope).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct DsseEnvelope {
    pub payload: String, // base64 encoded
    #[serde(rename = "payloadType")]
    pub payload_type: String,
    pub signatures: Vec<DsseSignatureEntry>,
}

/// The inner payload of a `WorkerBundle` that gets signed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerBundlePayload {
    /// The VM in which the worker must execute (e.g., "nxcc/workerd").
    pub vm: String,
    /// The executable code (e.g., JS, Python, WASM).
    #[serde(with = "serde_base64")]
    pub executable: Vec<u8>,
    /// Arbitrary metadata added by the publisher. Not interpreted by nXCC.
    pub metadata: HashMap<String, String>,
}

/// An executable `WorkerBundlePayload` wrapped in a DSSE envelope.
/// This struct holds the DSSE envelope as raw JSON bytes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerBundle(pub Vec<u8>);

/// The IANA media type for the WorkerBundlePayload when wrapped in DSSE.
pub const DSSE_WORKER_BUNDLE_PAYLOAD_TYPE: &str =
    "application/vnd.nxcc.workerbundlepayload.v1+json";

/// The IANA media type for the WorkOrderPayload when wrapped in DSSE.
pub const DSSE_WORK_ORDER_PAYLOAD_TYPE: &str = "application/vnd.nxcc.workorderpayload.v1+json";

impl WorkerBundle {
    /// Parses the DSSE envelope from the raw bytes of the WorkerBundle.
    fn dsse_envelope(&self) -> Result<DsseEnvelope, ConversionError> {
        serde_json::from_slice(&self.0).map_err(Into::into)
    }

    /// Retrieves the `WorkerBundlePayload` from the DSSE envelope.
    pub fn payload(&self) -> Result<WorkerBundlePayload, ConversionError> {
        let envelope = self.dsse_envelope()?;
        if envelope.payload_type != DSSE_WORKER_BUNDLE_PAYLOAD_TYPE {
            return Err(ConversionError::InvalidDssePayloadType {
                expected: DSSE_WORKER_BUNDLE_PAYLOAD_TYPE.to_string(),
                got: envelope.payload_type,
            });
        }
        let payload_bytes = BASE64_STANDARD.decode(&envelope.payload)?;
        serde_json::from_slice(&payload_bytes[..]).map_err(Into::into)
    }

    /// Calculates the SHA512 hash of the encoded `WorkerBundlePayload`.
    /// This hash is used for `ConsumerInfo.bundle_hash`.
    // TODO: remove this in favor of having the enclave verify the signer or having the hash of the executable be part of the signed data or something. right now it's totally broken, as the consumer cannot be verified with all of the arbitrary metadata in it
    pub fn hash_signed_payload(&self) -> Result<Vec<u8>, ConversionError> {
        use sha2::{Digest, Sha512};
        let payload_struct = self.payload()?;
        let payload_bytes = serde_json::to_vec(&payload_struct)?;
        Ok(Sha512::digest(payload_bytes).to_vec())
    }

    /// Extracts the first signature from the DSSE envelope.
    pub fn get_dsse_signature(&self) -> Result<Vec<u8>, ConversionError> {
        let envelope = self.dsse_envelope()?;
        if envelope.signatures.is_empty() {
            return Err(ConversionError::InvalidValue {
                field: "signatures".to_string(),
                message: "DSSE envelope has no signatures".to_string(),
            });
        }
        // Return the raw bytes of the first signature
        BASE64_STANDARD
            .decode(&envelope.signatures[0].sig)
            .map_err(Into::into)
    }
}

mod serde_base64 {
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&BASE64_STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        BASE64_STANDARD
            .decode(s.as_bytes())
            .map_err(serde::de::Error::custom)
    }
}

/// A structure combining a policy's `WorkerManifest` and its resolved `WorkerBundle`.
/// This replaces the old `PolicyBundle` for policy execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FullPolicyPackage {
    pub manifest: WorkerManifest,
    pub bundle: WorkerBundle,
}

/// The inner payload of a `WorkOrder` that gets signed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkOrderPayload {
    /// An arbitrary identifier for this work order.
    /// Useful for debugging and ensuring uniqueness when broadcasting over the p2p network.
    pub id: String,
    /// The worker to run, and its inputs and configuration.
    pub worker: WorkerManifest,
    /// Event listeners for the daemon to set up. The daemon will invoke the worker when they happen.
    pub events: Vec<WorkerEvent>,
}
