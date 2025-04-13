use interface::types::{AttestationReport, SecretId, SecretsBox};
use std::sync::Arc;

pub struct Secrets;

impl Secrets {
    pub fn new() -> Arc<Self> {
        Arc::new(Self)
    }

    pub fn get_report(&self, user_data: Vec<u8>) -> AttestationReport {
        AttestationReport {
            ephemeral_public_key: vec![],
            block_hashes: vec![],
            user_data,
        }
    }

    pub fn put_secrets(&self, bundles: Vec<(SecretsBox, AttestationReport)>) -> bool {
        true
    }

    pub fn get_secrets(
        &self,
        ids: Vec<SecretId>,
        policy_reports: Vec<(Vec<u8>, Vec<u8>)>,
        requester_ar: AttestationReport,
    ) -> SecretsBox {
        SecretsBox::new_empty()
    }

    pub fn check_secrets(&self, ids: Vec<SecretId>) -> Vec<(SecretId, bool, u64)> {
        ids.into_iter().map(|id| (id, true, 0)).collect()
    }
}
