use std::fmt;

use alloy_primitives::{Address, U256};
use serde::{Deserialize, Serialize};
use url::Url;

use super::error::ConversionError;
use crate::proto::interface;

/// Identifies a chain either by its numeric ID or by a custom gateway URL.
/// Custom gateways are treated as separate chains even if they have the same chain_id,
/// since we cannot verify that a custom gateway actually represents the intended chain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(untagged)]
pub enum ChainIdentifier {
    /// Standard chain identified by its numeric chain ID
    ChainId(u64),
    /// Custom chain identified by a gateway URL
    GatewayUrl(Url),
}

impl ChainIdentifier {
    /// Returns the chain ID if this is a ChainId variant, otherwise returns None
    pub fn chain_id(&self) -> Option<u64> {
        match self {
            ChainIdentifier::ChainId(id) => Some(*id),
            ChainIdentifier::GatewayUrl(_) => None,
        }
    }

    /// Returns the gateway URL if this is a GatewayUrl variant, otherwise returns None
    pub fn gateway_url(&self) -> Option<&Url> {
        match self {
            ChainIdentifier::ChainId(_) => None,
            ChainIdentifier::GatewayUrl(url) => Some(url),
        }
    }
}

impl Default for ChainIdentifier {
    fn default() -> Self {
        ChainIdentifier::ChainId(0)
    }
}

impl fmt::Display for ChainIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChainIdentifier::ChainId(id) => write!(f, "{}", id),
            ChainIdentifier::GatewayUrl(url) => write!(f, "{}", url),
        }
    }
}

impl TryFrom<interface::ChainIdentifier> for ChainIdentifier {
    type Error = ConversionError;
    fn try_from(p: interface::ChainIdentifier) -> Result<Self, Self::Error> {
        match p.identifier {
            Some(interface::chain_identifier::Identifier::ChainId(id)) => {
                Ok(ChainIdentifier::ChainId(id))
            }
            Some(interface::chain_identifier::Identifier::GatewayUrl(url)) => {
                let parsed_url = Url::parse(&url).map_err(|e| ConversionError::InvalidValue {
                    field: "gateway_url".to_string(),
                    message: e.to_string(),
                })?;
                Ok(ChainIdentifier::GatewayUrl(parsed_url))
            }
            None => Err(ConversionError::MissingField("identifier".to_string())),
        }
    }
}

impl From<ChainIdentifier> for interface::ChainIdentifier {
    fn from(value: ChainIdentifier) -> Self {
        let identifier = match value {
            ChainIdentifier::ChainId(id) => interface::chain_identifier::Identifier::ChainId(id),
            ChainIdentifier::GatewayUrl(url) => {
                interface::chain_identifier::Identifier::GatewayUrl(url.to_string())
            }
        };
        interface::ChainIdentifier {
            identifier: Some(identifier),
        }
    }
}

impl From<&ChainIdentifier> for interface::ChainIdentifier {
    fn from(value: &ChainIdentifier) -> Self {
        let identifier = match value {
            ChainIdentifier::ChainId(id) => interface::chain_identifier::Identifier::ChainId(*id),
            ChainIdentifier::GatewayUrl(url) => {
                interface::chain_identifier::Identifier::GatewayUrl(url.to_string())
            }
        };
        interface::ChainIdentifier {
            identifier: Some(identifier),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SecretId {
    pub chain: ChainIdentifier,
    pub identity_address: Address,
    pub identity_id: U256,
}

impl TryFrom<interface::SecretIdentifier> for SecretId {
    type Error = ConversionError;
    fn try_from(p: interface::SecretIdentifier) -> Result<Self, Self::Error> {
        Ok(Self {
            chain: p
                .chain
                .ok_or(ConversionError::MissingField("chain".to_string()))?
                .try_into()?,
            identity_address: p.identity_address.parse().map_err(
                |e: alloy_primitives::hex::FromHexError| ConversionError::InvalidValue {
                    field: "identity_address".to_string(),
                    message: e.to_string(),
                },
            )?,
            identity_id: p.identity_id.parse().map_err(
                |e: alloy_primitives::ruint::ParseError| ConversionError::InvalidValue {
                    field: "identity_id".to_string(),
                    message: e.to_string(),
                },
            )?,
        })
    }
}

impl From<SecretId> for interface::SecretIdentifier {
    fn from(value: SecretId) -> Self {
        interface::SecretIdentifier {
            chain: Some(value.chain.into()),
            identity_address: format!("{:#x}", value.identity_address),
            identity_id: value.identity_id.to_string(),
        }
    }
}

impl From<&SecretId> for interface::SecretIdentifier {
    fn from(value: &SecretId) -> Self {
        interface::SecretIdentifier {
            chain: Some(value.chain.clone().into()),
            identity_address: format!("{:#x}", value.identity_address),
            identity_id: value.identity_id.to_string(),
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ConsumerInfo {
    pub bundle_hash: Vec<u8>,
    pub signature: Vec<u8>,
}

impl From<interface::ConsumerInfo> for ConsumerInfo {
    fn from(p: interface::ConsumerInfo) -> Self {
        Self {
            bundle_hash: p.bundle_hash,
            signature: p.signature,
        }
    }
}

impl From<ConsumerInfo> for interface::ConsumerInfo {
    fn from(value: ConsumerInfo) -> Self {
        interface::ConsumerInfo {
            bundle_hash: value.bundle_hash,
            signature: value.signature,
        }
    }
}

impl From<&ConsumerInfo> for interface::ConsumerInfo {
    fn from(value: &ConsumerInfo) -> Self {
        interface::ConsumerInfo {
            bundle_hash: value.bundle_hash.clone(),
            signature: value.signature.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretRequest {
    pub secret_id: SecretId,
    pub consumer: ConsumerInfo,
}

impl TryFrom<interface::SecretRequest> for SecretRequest {
    type Error = ConversionError;
    fn try_from(p: interface::SecretRequest) -> Result<Self, Self::Error> {
        Ok(Self {
            secret_id: p
                .secret_id
                .ok_or(ConversionError::MissingField("secret_id".to_string()))?
                .try_into()?,
            consumer: p
                .consumer
                .map(ConsumerInfo::from)
                .ok_or(ConversionError::MissingField("consumer".to_string()))?,
        })
    }
}

impl From<SecretRequest> for interface::SecretRequest {
    fn from(value: SecretRequest) -> Self {
        interface::SecretRequest {
            secret_id: Some(value.secret_id.into()),
            consumer: Some(value.consumer.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretsBox {
    pub encrypted_payload: Vec<u8>,
    pub sender_public_key: Vec<u8>, // This is the sender's *ephemeral key exchange* public key
    pub alg: String,
    pub contained_secret_ids: Vec<SecretId>,
}

impl SecretsBox {
    pub fn new_empty() -> Self {
        Self {
            encrypted_payload: vec![],
            sender_public_key: vec![],
            alg: "X25519_AES-GCM-SIV_Ed25519".to_string(), // Default algorithm
            contained_secret_ids: vec![],
        }
    }

    pub fn calculate_binding_hash(&self) -> [u8; 32] {
        use sha2::Digest as _;
        let mut hasher = sha2::Sha256::default();
        hasher.update(&self.encrypted_payload);
        hasher.update(&self.sender_public_key);
        hasher.update(self.alg.as_bytes());
        // Hash contained IDs consistently (sort them first)
        let mut sorted_ids = self.contained_secret_ids.clone();
        sorted_ids.sort();
        let mut id_bytes = Vec::new();
        ciborium::into_writer(&sorted_ids, &mut id_bytes).unwrap();
        hasher.update(&id_bytes);
        hasher.finalize().into()
    }
}

impl TryFrom<interface::SecretsBox> for SecretsBox {
    type Error = ConversionError;
    fn try_from(p: interface::SecretsBox) -> Result<Self, Self::Error> {
        Ok(Self {
            encrypted_payload: p.encrypted_payload,
            sender_public_key: p.sender_public_key,
            alg: p.alg,
            contained_secret_ids: p
                .contained_secret_ids
                .into_iter()
                .map(SecretId::try_from)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

impl From<SecretsBox> for interface::SecretsBox {
    fn from(value: SecretsBox) -> Self {
        interface::SecretsBox {
            encrypted_payload: value.encrypted_payload,
            sender_public_key: value.sender_public_key,
            alg: value.alg,
            contained_secret_ids: value
                .contained_secret_ids
                .into_iter()
                .map(Into::into)
                .collect(),
        }
    }
}

impl From<&SecretsBox> for interface::SecretsBox {
    fn from(value: &SecretsBox) -> Self {
        interface::SecretsBox {
            encrypted_payload: value.encrypted_payload.clone(),
            sender_public_key: value.sender_public_key.clone(),
            alg: value.alg.clone(),
            contained_secret_ids: value
                .contained_secret_ids
                .iter()
                .cloned()
                .map(Into::into)
                .collect(),
        }
    }
}
