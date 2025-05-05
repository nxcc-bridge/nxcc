use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

use nxcc_interface::{
    policy::{PolicyBundle, PolicyManifest},
    types::SecretId,
};
use tokio::sync::RwLock;
use tracing::{debug, error, info, trace, warn};

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
            warn!("Using local mock policy for secret {:?}", secret_id);
            // Load from a fixed local path relative to the Cargo manifest dir
            let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or("..".to_string());
            let policy_bundle_path = Path::new(&manifest_dir).join("tests/policy/mock_policy.json");
            let worker_code_path_rel = "tests/policy/mock_worker.js"; // Path relative to manifest dir

            debug!(
                "Loading mock policy bundle from: {}",
                policy_bundle_path.display()
            );

            let policy_bundle_content = tokio::fs::read_to_string(&policy_bundle_path)
                .await
                .map_err(|e| AppError::Io(e))?;

            // Deserialize only the manifest part first
            #[derive(Debug, Clone, serde::Deserialize)]
            pub struct MockPolicyBundle {
                pub manifest: nxcc_interface::policy::PolicyManifest,
                #[serde(rename = "executable")]
                pub executable_path: String,
            }

            let mut policy_bundle: MockPolicyBundle = serde_json::from_str(&policy_bundle_content)
                .map_err(|e| {
                    AppError::Service(format!("Failed to parse mock policy JSON: {}", e))
                })?;

            // Load the executable code based on the relative path in the manifest
            let worker_code_path_abs = PathBuf::from(manifest_dir).join(worker_code_path_rel);
            debug!(
                "Loading mock worker code from: {}",
                worker_code_path_abs.display()
            );
            let worker_code = tokio::fs::read(&worker_code_path_abs)
                .await
                .map_err(|e| AppError::Io(e))?;

            // Validate the loaded manifest
            self.manifest_checker
                .check_manifest(&policy_bundle.manifest)?;
            debug!(
                "Manifest check passed for mock policy of secret {:?}",
                secret_id
            );

            // Return the fully constructed bundle
            return Ok(PolicyBundle {
                manifest: policy_bundle.manifest,
                executable: worker_code,
            });
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
