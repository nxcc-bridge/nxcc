use serde::{Deserialize, Serialize};

use super::{
    attestation::{
        AttestationBundle, EnvReport, OperatorSignature, RawAttestation, StandardizedClaims,
    },
    error::ConversionError,
    secrets::{ConsumerInfo, SecretId},
};
use crate::proto::interface;

/// Sanitized attestation bundle for policy workers, excluding system userdata
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyAttestationBundle {
    /// Platform type for the attestation
    pub platform_type: String,
    /// Raw attestation evidence
    pub evidence: Vec<u8>,
    /// User-provided data only (no ephemeral keys, no block hashes)
    pub user_data: Vec<u8>,
}

/// Sanitized environment report for policy workers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyEnvReport {
    pub attestation: PolicyAttestationBundle,
    pub operator_signature: Option<OperatorSignature>,
}

impl PolicyEnvReport {
    /// Create a sanitized policy environment report from a full environment report
    /// This removes system userdata (ephemeral keys, block hashes) while preserving
    /// user data and platform measurements for policy decisions
    pub fn from_env_report(env_report: &EnvReport, user_provided_data: Vec<u8>) -> Self {
        Self {
            attestation: PolicyAttestationBundle {
                platform_type: env_report.attestation.raw_attestation.platform_type.clone(),
                evidence: env_report.attestation.raw_attestation.evidence.clone(),
                user_data: user_provided_data,
            },
            operator_signature: env_report.operator_signature.clone(),
        }
    }

    /// Convert back to a full EnvReport for protobuf serialization
    /// Note: This reconstructs minimal system fields for compatibility
    pub fn to_env_report(&self) -> EnvReport {
        EnvReport {
            attestation: AttestationBundle {
                raw_attestation: RawAttestation {
                    platform_type: self.attestation.platform_type.clone(),
                    evidence: self.attestation.evidence.clone(),
                    certificates: None, // Empty - system data removed
                },
                detached_userdata: self.attestation.user_data.clone(),
            },
            operator_signature: self.operator_signature.clone(),
        }
    }
}

/// A request for the policy runner that references multiple secrets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyExecutionRequest {
    pub secret_ids: Vec<SecretId>,
    pub consumer: ConsumerInfo,
    pub env_report: EnvReport, // The EnvReport of the entity being evaluated
    /// Standardized attestation claims extracted from the verified env_report
    /// These are available when the attestation system successfully verifies the report
    pub attestation_claims: Option<StandardizedClaims>,
}

impl PolicyExecutionRequest {
    /// Create a sanitized version for policy worker execution
    /// This removes system userdata while preserving user data and claims
    pub fn for_policy_worker(
        &self,
        user_provided_data: Vec<u8>,
    ) -> PolicyExecutionContextForWorker {
        PolicyExecutionContextForWorker {
            secret_ids: self.secret_ids.clone(),
            consumer: self.consumer.clone(),
            env_report: PolicyEnvReport::from_env_report(&self.env_report, user_provided_data),
            attestation_claims: self.attestation_claims.clone(),
        }
    }
}

/// Sanitized context sent to policy workers (excludes system userdata)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyExecutionContextForWorker {
    pub secret_ids: Vec<SecretId>,
    pub consumer: ConsumerInfo,
    pub env_report: PolicyEnvReport, // Sanitized EnvReport without system userdata
    /// Standardized attestation claims extracted from the verified env_report
    pub attestation_claims: Option<StandardizedClaims>,
}

impl TryFrom<interface::PolicyExecutionRequest> for PolicyExecutionRequest {
    type Error = ConversionError;
    fn try_from(p: interface::PolicyExecutionRequest) -> Result<Self, Self::Error> {
        Ok(Self {
            secret_ids: p
                .secret_ids
                .into_iter()
                .map(SecretId::try_from)
                .collect::<Result<Vec<_>, _>>()?,
            consumer: p
                .consumer
                .map(ConsumerInfo::from)
                .ok_or(ConversionError::MissingField("consumer".to_string()))?,
            env_report: p
                .env_report
                .ok_or(ConversionError::MissingField("env_report".to_string()))?
                .try_into()?,
            attestation_claims: None, // Populated by enclave after verification
        })
    }
}

impl From<PolicyExecutionRequest> for interface::PolicyExecutionRequest {
    fn from(value: PolicyExecutionRequest) -> Self {
        interface::PolicyExecutionRequest {
            secret_ids: value.secret_ids.into_iter().map(Into::into).collect(),
            consumer: Some(value.consumer.into()),
            env_report: Some(value.env_report.into()),
        }
    }
}

impl From<&PolicyExecutionRequest> for interface::PolicyExecutionRequest {
    fn from(value: &PolicyExecutionRequest) -> Self {
        interface::PolicyExecutionRequest {
            secret_ids: value.secret_ids.iter().cloned().map(Into::into).collect(),
            consumer: Some(value.consumer.clone().into()),
            env_report: Some(value.env_report.clone().into()),
        }
    }
}

/// The runner's final judgment about a request. This structure is used internally within the enclave
/// between the runner and secrets service. It's distinct from the proto message used for gRPC transport.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyExecutionReport {
    pub request: PolicyExecutionRequest,
    pub decision: bool,
    pub timestamp: u64, // Unix timestamp
}
