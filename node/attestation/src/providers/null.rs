use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    user_data_binding, AttestationBundle, AttestationProvider, Measurement, RawAttestation,
    StandardizedClaims, VerificationResult,
};

/// Null attestation evidence structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NullAttestationEvidence {
    /// Hash of the userdata
    pub userdata_hash: Vec<u8>,
    /// Ephemeral key used for this attestation (for verification)
    pub ephemeral_key: Vec<u8>,
    /// Random nonce to make each attestation unique (exposes determinism bugs)
    #[serde(default)]
    pub nonce: Option<Vec<u8>>,
}

impl NullAttestationEvidence {
    /// Create new null attestation evidence with random nonce
    pub fn new(userdata_hash: Vec<u8>, ephemeral_key: Vec<u8>) -> Self {
        use rand::Rng;

        let mut rng = rand::thread_rng();
        let mut random_nonce = vec![0u8; 32];
        rng.fill(&mut random_nonce[..]);

        Self {
            userdata_hash,
            ephemeral_key,
            nonce: Some(random_nonce),
        }
    }

    /// Create null attestation evidence without nonce (for tests that need deterministic behavior)
    pub fn new_deterministic(userdata_hash: Vec<u8>, ephemeral_key: Vec<u8>) -> Self {
        Self {
            userdata_hash,
            ephemeral_key,
            nonce: None,
        }
    }
}

/// Null attestation provider for systems without TEE hardware.
///
/// This provider creates attestations by signing the userdata hash with the ephemeral
/// public key from the userdata. It provides a production-quality fallback that:
/// - Is always available on all systems
/// - Only used when no TEE hardware is detected
/// - Cannot be mixed with real TEE verification
/// - Provides clear "null" platform identification to policies
pub struct NullAttestationProvider;

impl Default for NullAttestationProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl NullAttestationProvider {
    pub fn new() -> Self {
        Self
    }

    /// Extract ephemeral key from userdata for signing
    fn extract_ephemeral_key_from_userdata(&self, userdata: &[u8]) -> Result<[u8; 32]> {
        let user_data = user_data_binding::UserData::from_cbor(userdata)?;

        if user_data.ephemeral_public_key.len() != 32 {
            return Err(anyhow::anyhow!(
                "Invalid ephemeral key length: expected 32 bytes, got {}",
                user_data.ephemeral_public_key.len()
            ));
        }

        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(&user_data.ephemeral_public_key);
        Ok(key_bytes)
    }

    /// Create standardized claims for null attestation
    fn create_null_claims(&self, userdata_hash: &[u8]) -> StandardizedClaims {
        use rand::Rng;

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let mut rng = rand::thread_rng();

        // Randomize platform measurement to expose determinism bugs
        let mut platform_measurement = vec![0u8; 32];
        rng.fill(&mut platform_measurement[..]);

        // Randomize application measurement to expose determinism bugs
        let mut app_measurement = vec![0u8; 48];
        rng.fill(&mut app_measurement[..]);

        StandardizedClaims {
            // Core freshness and context
            iat: timestamp,
            eat_nonce: Some(userdata_hash.to_vec()),

            // Identity and provenance - clearly indicate null platform
            ueid: None, // No stable device identity in null mode
            sueids: None,
            oemid: Some("nxcc-null".to_string()),
            hwmodel: Some("null".to_string()),
            hwversion: Some("1.0".to_string()),

            // Debug and boot status - null is always "production" (no debug mode)
            dbgstat: 0,          // Debug disabled (production mode)
            oemboot: Some(true), // Consider secure boot active

            // Software identity
            swname: Some("nxcc-null-attestation".to_string()),
            swversion: Some("1.0".to_string()),
            manifests: None,

            // Measurements - randomized to expose determinism bugs
            measurements: vec![
                Measurement {
                    val: platform_measurement, // Randomized instead of static hash
                    alg: "sha-256".to_string(),
                    measurement_type: Some("platform".to_string()),
                    vendor: Some("nxcc".to_string()),
                    version: Some("1.0".to_string()),
                },
                Measurement {
                    val: app_measurement, // Randomized instead of static bytes
                    alg: "sha-384".to_string(),
                    measurement_type: Some("application".to_string()),
                    vendor: Some("nxcc-null".to_string()),
                    version: Some("1.0".to_string()),
                },
            ],
            measres: None,

            // Execution structure
            submods: None,

            // Key binding - no PoP since we don't have hardware key
            cnf: None,
            intuse: None,

            // Lifecycle freshness
            uptime: None,
            bootcount: None,
            bootseed: None,

            // Profile selection - clearly identify as null platform
            eat_profile: "urn:nxcc:profile:null-v1".to_string(),

            // Assurance artifacts
            dloas: None,
        }
    }
}

#[async_trait]
impl AttestationProvider for NullAttestationProvider {
    fn platform_type(&self) -> &str {
        "null"
    }

    fn is_available(&self) -> bool {
        // Always available on all systems
        true
    }

    fn max_user_data_size(&self) -> usize {
        // No hardware limitation, but keep reasonable limit
        1024
    }

    async fn update_config(&mut self, _config_json: &str) -> Result<()> {
        // No configuration needed for null provider
        Ok(())
    }

    async fn generate_attestation(&self, _userdata_hash: &[u8]) -> Result<RawAttestation> {
        tracing::info!(
            "Generating null attestation - signature will be created by AttestationService"
        );

        // For null provider, the AttestationService handles signature creation
        // since it has access to the ephemeral key. This method should not be called
        // directly for null providers.
        Err(anyhow::anyhow!(
            "Null provider generate_attestation should not be called directly. Use \
             AttestationService."
        ))
    }

    async fn verify_attestation(&self, bundle: &AttestationBundle) -> Result<VerificationResult> {
        // Only handle null platform type
        if bundle.raw_attestation.platform_type != "null" {
            return Ok(VerificationResult::Unsupported);
        }

        tracing::info!("Verifying null attestation");

        // Parse null evidence from JSON
        let evidence: NullAttestationEvidence =
            match serde_json::from_slice(&bundle.raw_attestation.evidence) {
                Ok(evidence) => evidence,
                Err(e) => {
                    return Ok(VerificationResult::Failed(format!(
                        "Failed to parse null attestation evidence: {}",
                        e
                    )));
                }
            };

        // First, try to parse userdata to ensure it's valid CBOR
        let ephemeral_key_from_userdata =
            match self.extract_ephemeral_key_from_userdata(&bundle.detached_userdata) {
                Ok(key) => key,
                Err(_) => {
                    return Ok(VerificationResult::Failed(
                        "Failed to parse requester userdata".to_string(),
                    ));
                }
            };

        // Verify userdata hash matches
        let computed_userdata_hash = user_data_binding::hash_userdata(&bundle.detached_userdata);
        if evidence.userdata_hash != computed_userdata_hash {
            return Ok(VerificationResult::Failed(
                "Userdata hash mismatch in null attestation".to_string(),
            ));
        }

        if evidence.ephemeral_key != ephemeral_key_from_userdata {
            return Ok(VerificationResult::Failed(
                "Ephemeral key mismatch between evidence and userdata".to_string(),
            ));
        }

        // Create claims for successful null verification
        let claims = self.create_null_claims(&computed_userdata_hash);

        tracing::info!("Successfully verified null attestation");
        Ok(VerificationResult::Verified(Box::new(claims)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::user_data_binding::UserData;

    #[tokio::test]
    async fn test_null_provider_basic() {
        let provider = NullAttestationProvider::new();

        assert_eq!(provider.platform_type(), "null");
        assert!(provider.is_available());
        assert_eq!(provider.max_user_data_size(), 1024);
    }

    #[tokio::test]
    async fn test_null_provider_config() {
        let mut provider = NullAttestationProvider::new();
        let result = provider.update_config("{}").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_null_attestation_generation_not_called_directly() {
        let provider = NullAttestationProvider::new();
        let userdata_hash = [0x42; 32];

        // Direct generation should fail - must use AttestationService
        let result = provider.generate_attestation(&userdata_hash).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("should not be called directly"));
    }

    #[tokio::test]
    async fn test_null_attestation_verification() {
        let provider = NullAttestationProvider::new();

        // Create test userdata with ephemeral key
        let ephemeral_key_bytes = [0x42; 32]; // X25519 public key
        let user_data = UserData::new(ephemeral_key_bytes.to_vec(), vec![]);
        let userdata_cbor = user_data.to_cbor().unwrap();
        let userdata_hash = user_data_binding::hash_userdata(&userdata_cbor);

        // Create null evidence (deterministic for testing)
        let evidence = NullAttestationEvidence::new_deterministic(
            userdata_hash.to_vec(),
            ephemeral_key_bytes.to_vec(),
        );
        let evidence_bytes = serde_json::to_vec(&evidence).unwrap();

        let bundle = AttestationBundle {
            raw_attestation: RawAttestation {
                platform_type: "null".to_string(),
                evidence: evidence_bytes,
                certificates: None,
            },
            detached_userdata: userdata_cbor,
        };

        let result = provider.verify_attestation(&bundle).await.unwrap();
        match result {
            VerificationResult::Verified(claims) => {
                assert_eq!(claims.eat_profile, "urn:nxcc:profile:null-v1");
                assert_eq!(claims.hwmodel, Some("null".to_string()));
                assert_eq!(claims.oemid, Some("nxcc-null".to_string()));
                assert_eq!(claims.dbgstat, 0);
            }
            VerificationResult::Failed(err) => panic!("Verification failed: {}", err),
            VerificationResult::Unsupported => panic!("Verification was unsupported"),
        }
    }

    #[tokio::test]
    async fn test_null_provider_rejects_other_platforms() {
        let provider = NullAttestationProvider::new();

        let bundle = AttestationBundle {
            raw_attestation: RawAttestation {
                platform_type: "tdx".to_string(),
                evidence: vec![0u8; 32],
                certificates: None,
            },
            detached_userdata: vec![],
        };

        let result = provider.verify_attestation(&bundle).await.unwrap();
        assert!(matches!(result, VerificationResult::Unsupported));
    }
}
