use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

use interface::{
    policy::{PolicyBundle, PolicyManifest},
    types::SecretId,
};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::{config::Config, error::AppError, web3::gateways::GatewayManager};

#[derive(Clone, Copy)]
pub struct ManifestChecker;

impl ManifestChecker {
    pub fn check_manifest(&self, manifest: &PolicyManifest) -> Result<(), AppError> {
        // This is a dummy implementation that just logs the manifest and accepts it
        debug!("Checking policy manifest: {}", manifest.name);

        // In a real implementation, we would check:
        // - Version compatibility
        // - Resource constraints
        // - Security policies
        // - Signature verification

        Ok(())
    }
}

#[derive(Clone)]
pub struct PolicyManager {
    gateway_manager: GatewayManager,
    manifest_checker: ManifestChecker,
    memory_cache: Arc<RwLock<HashMap<SecretId, PolicyBundle>>>,
    disk_cache_path: Option<PathBuf>,
}

impl PolicyManager {
    pub async fn new(gateway_manager: GatewayManager, config: &Config) -> Result<Self, AppError> {
        let disk_cache_path = match &config.policy_cache_dir {
            Some(path) => Some(path.clone()),
            None => {
                let sys_temp = std::env::temp_dir();
                let path = sys_temp.join("nxcc_policy_cache");
                Some(path)
            }
        };

        if let Some(path) = &disk_cache_path {
            tokio::fs::create_dir_all(path)
                .await
                .map_err(AppError::Io)?;
            info!("Using policy disk cache at: {}", path.display());
        } else {
            info!("Policy disk cache disabled.");
        }

        Ok(Self {
            gateway_manager,
            manifest_checker: ManifestChecker,
            memory_cache: Arc::new(RwLock::new(HashMap::new())),
            disk_cache_path,
        })
    }

    /// Fetches policy from cache (memory/disk) or network, validates manifest.
    pub async fn get_policy(&self, secret_id: &SecretId) -> Result<PolicyBundle, AppError> {
        // 1. Check memory cache
        if let Some(policy) = self.memory_cache.read().await.get(secret_id) {
            debug!("Policy memory cache hit for secret {:?}", secret_id);
            return Ok(policy.clone());
        }
        debug!("Policy memory cache miss for secret {:?}", secret_id);

        // 2. Check disk cache
        if let Some(policy) = self.load_from_disk(secret_id).await? {
            debug!("Policy disk cache hit for secret {:?}", secret_id);
            // Validate manifest before returning from disk cache
            self.manifest_checker.check_manifest(&policy.manifest)?;
            // Add to memory cache
            self.memory_cache
                .write()
                .await
                .insert(secret_id.clone(), policy.clone());
            return Ok(policy);
        }
        debug!("Policy disk cache miss for secret {:?}", secret_id);

        // 3. Fetch from network
        let policy = self.fetch_from_network(secret_id).await?;

        // 4. Validate manifest
        self.manifest_checker.check_manifest(&policy.manifest)?;
        debug!("Manifest check passed for policy of secret {:?}", secret_id);

        // 5. Store in caches
        self.store_to_disk(secret_id, &policy).await?;
        self.memory_cache
            .write()
            .await
            .insert(secret_id.clone(), policy.clone());
        info!(
            "Successfully fetched, validated, and cached policy for {:?}",
            secret_id
        );

        Ok(policy)
    }

    async fn fetch_from_network(&self, secret_id: &SecretId) -> Result<PolicyBundle, AppError> {
        let policy_url = self
            .gateway_manager
            .get_policy_url(
                secret_id.chain_id,
                secret_id.identity_address,
                secret_id.identity_id,
            )
            .await?;

        info!(
            "Fetching policy for secret {:?} from URL: {}",
            secret_id, policy_url
        );

        // Handle mock URLs for testing/dev
        if policy_url.starts_with("mock://") {
            warn!("Using mock policy for secret {:?}", secret_id);
            // Create a dummy policy bundle
            let mock_manifest = PolicyManifest {
                version: "1.0".to_string(),
                name: format!("Mock Policy for {:?}", secret_id),
                description: "A dummy policy for testing".to_string(),
                allowed_consumers: vec![], // Adjust as needed
                execution_constraints: interface::policy::ExecutionConstraints {
                    max_memory_mb: 128,
                    max_execution_time_ms: 1000,
                    allowed_network_calls: false,
                },
            };
            let mock_policy = PolicyBundle {
                manifest: mock_manifest,
                executable: b"mock_executable_code".to_vec(),
            };
            return Ok(mock_policy);
        }

        // Fetch the actual policy content
        let client = reqwest::Client::new();
        let response = client.get(&policy_url).send().await.map_err(|e| {
            AppError::Network(format!("Failed to fetch policy from {}: {}", policy_url, e))
        })?;

        if !response.status().is_success() {
            return Err(AppError::Network(format!(
                "Failed to fetch policy from {}: Status {}",
                policy_url,
                response.status()
            )));
        }

        // Assume JSON for now, might need to handle different formats (e.g., raw binary)
        let policy: PolicyBundle = response.json().await.map_err(|e| {
            AppError::Service(format!(
                "Failed to parse policy JSON from {}: {}",
                policy_url, e
            ))
        })?;

        Ok(policy)
    }

    fn get_cache_filepath(&self, secret_id: &SecretId) -> Option<PathBuf> {
        self.disk_cache_path.as_ref().map(|base_path| {
            let mut hasher = DefaultHasher::new();
            secret_id.hash(&mut hasher);
            let filename = format!("{:x}.policy", hasher.finish());
            base_path.join(filename)
        })
    }

    async fn load_from_disk(&self, secret_id: &SecretId) -> Result<Option<PolicyBundle>, AppError> {
        if let Some(filepath) = self.get_cache_filepath(secret_id) {
            if !filepath.exists() {
                return Ok(None);
            }

            match tokio::fs::read(&filepath).await {
                Ok(data) => {
                    // Using ciborium for CBOR serialization instead of bincode
                    match ciborium::from_reader::<PolicyBundle, _>(&data[..]) {
                        Ok(policy) => {
                            // TODO: Add cache expiry check?
                            Ok(Some(policy))
                        }
                        Err(e) => {
                            error!(
                                "Failed to deserialize policy from disk cache {}: {:?}. Removing \
                                 file.",
                                filepath.display(),
                                e
                            );
                            // Remove corrupted file
                            let _ = tokio::fs::remove_file(&filepath).await;
                            Ok(None)
                        }
                    }
                }
                Err(e) => {
                    error!(
                        "Failed to read policy from disk cache {}: {}",
                        filepath.display(),
                        e
                    );
                    Ok(None) // Treat read error as cache miss
                }
            }
        } else {
            Ok(None) // Disk cache disabled
        }
    }

    async fn store_to_disk(
        &self,
        secret_id: &SecretId,
        policy: &PolicyBundle,
    ) -> Result<(), AppError> {
        if let Some(filepath) = self.get_cache_filepath(secret_id) {
            // Using Vec as a temporary buffer for serialization
            let mut data = Vec::new();
            match ciborium::into_writer(policy, &mut data) {
                Ok(()) => {
                    if let Err(e) = tokio::fs::write(&filepath, data).await {
                        error!(
                            "Failed to write policy to disk cache {}: {}",
                            filepath.display(),
                            e
                        );
                        // Don't treat write failure as fatal, just log it
                    } else {
                        debug!("Stored policy for {:?} to disk cache.", secret_id);
                    }
                }
                Err(e) => {
                    error!("Failed to serialize policy for disk cache: {:?}", e);
                }
            }
        }
        Ok(())
    }
}
