use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

use nxcc_interface::types::{
    FullPolicyPackage, IdentityId, SecretId, WorkerBundle, WorkerBundlePayload,
    WorkerBundlePointer, WorkerManifest,
};
use tokio::sync::RwLock;
use tracing::{debug, error, info, trace, warn};

use crate::{config::Config, error::AppError, web3::gateways::GatewayManager};

#[derive(Clone)]
pub struct PolicyManager {
    gateway_manager: GatewayManager,
    memory_cache: Arc<RwLock<HashMap<SecretId, FullPolicyPackage>>>,
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
            memory_cache: Arc::new(RwLock::new(HashMap::new())),
            disk_cache_path,
        })
    }

    /// Fetches a policy, which consists of a `WorkerManifest` and its corresponding `WorkerBundle`.
    /// The `WorkerManifest` (the policy itself) is validated to ensure it requests no identities.
    pub async fn get_policy(&self, secret_id: &SecretId) -> Result<FullPolicyPackage, AppError> {
        // 1. Check memory cache
        if let Some(policy) = self.memory_cache.read().await.get(secret_id) {
            debug!("Policy memory cache hit for secret {:?}", secret_id);
            return Ok(policy.clone());
        }
        debug!("Policy memory cache miss for secret {:?}", secret_id);

        // 2. Check disk cache
        if let Some(package) = self.load_from_disk(secret_id).await? {
            debug!("Policy disk cache hit for secret {:?}", secret_id);
            // Add to memory cache
            self.memory_cache
                .write()
                .await
                .insert(secret_id.clone(), package.clone());
            return Ok(package);
        }
        debug!("Policy disk cache miss for secret {:?}", secret_id);

        // 3. Fetch manifest from network (this is the "policy")
        let manifest_url = self
            .gateway_manager
            .get_policy_url(
                secret_id.chain_id,
                secret_id.identity_address,
                secret_id.identity_id,
            )
            .await?;

        let manifest = self.fetch_worker_manifest(&manifest_url, secret_id).await?;

        // 4. Validate policy manifest (must not request identities)
        if !manifest.identities.is_empty() {
            error!(
                "Policy manifest for {:?} is invalid: must not request identities. Found: {:?}",
                secret_id, manifest.identities
            );
            return Err(AppError::Service(format!(
                "Policy for {:?} cannot request identities",
                secret_id
            )));
        }

        // 5. Fetch the worker bundle using the pointer in the manifest
        let bundle = self
            .fetch_worker_bundle(&manifest.bundle, &manifest_url, secret_id)
            .await?;

        let package = FullPolicyPackage { manifest, bundle };

        // 6. Store in caches
        self.store_to_disk(secret_id, &package).await?;
        self.memory_cache
            .write()
            .await
            .insert(secret_id.clone(), package.clone());
        info!(
            "Successfully fetched, validated, and cached policy for {:?}",
            secret_id
        );

        Ok(package)
    }

    async fn fetch_worker_manifest(
        &self,
        manifest_url: &str,
        secret_id_for_log: &SecretId, // For logging context if URL is mock
    ) -> Result<WorkerManifest, AppError> {
        info!(
            "Fetching worker manifest for policy {:?} from URL: {}",
            secret_id_for_log, manifest_url
        );

        // Handle mock URLs for testing/dev
        if manifest_url.starts_with("mock://") {
            warn!(
                "Using local mock worker manifest for policy {:?}",
                secret_id_for_log
            );
            // Load from a fixed local path relative to the Cargo manifest dir
            let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or("..".to_string());
            let mock_manifest_path = Path::new(&manifest_dir).join("tests/policy/mock_policy.json");

            debug!(
                "Loading mock worker manifest from: {}",
                mock_manifest_path.display()
            );

            let manifest_content = tokio::fs::read_to_string(&mock_manifest_path)
                .await
                .map_err(AppError::Io)?;

            let manifest: WorkerManifest =
                serde_json::from_str(&manifest_content).map_err(|e| {
                    AppError::Service(format!("Failed to parse mock worker manifest JSON: {}", e))
                })?;
            return Ok(manifest);
        }

        // Fetch the actual policy content
        let response = reqwest::get(manifest_url).await.map_err(|e| {
            AppError::Network(format!(
                "Failed to fetch worker manifest from {}: {}",
                manifest_url, e
            ))
        })?;

        if !response.status().is_success() {
            return Err(AppError::Network(format!(
                "Failed to fetch worker manifest from {}: Status {}",
                manifest_url,
                response.status()
            )));
        }

        let manifest: WorkerManifest = response.json().await.map_err(|e| {
            AppError::Service(format!(
                "Failed to parse worker manifest JSON from {}: {}",
                manifest_url, e
            ))
        })?;

        Ok(manifest)
    }

    async fn fetch_worker_bundle(
        &self,
        bundle_pointer: &WorkerBundlePointer,
        manifest_url_for_context: &str, // Used to resolve relative file URLs for mocks
        secret_id_for_log: &SecretId,   // For logging context
    ) -> Result<WorkerBundle, AppError> {
        let bundle_url_str = bundle_pointer.source.as_str();
        info!(
            "Fetching worker bundle for policy {:?} from URL: {}",
            secret_id_for_log, bundle_url_str
        );

        let executable_bytes = if bundle_pointer.source.scheme() == "file" {
            // Handle local file paths, potentially relative for mock scenarios
            let path_str = bundle_pointer.source.path();
            let path = PathBuf::from(path_str.strip_prefix('/').unwrap_or(path_str)); // Handle absolute file paths

            let absolute_path = if path.is_absolute() {
                path
            } else {
                // For relative paths in mock, resolve against CARGO_MANIFEST_DIR or manifest_url context
                // Assuming mock_policy.json specifies relative paths like "tests/policy/mock_worker.js"
                // and manifest_url_for_context is something like "mock://..."
                if manifest_url_for_context.starts_with("mock://") {
                    let manifest_dir =
                        std::env::var("CARGO_MANIFEST_DIR").unwrap_or("..".to_string());
                    PathBuf::from(manifest_dir).join(path)
                } else {
                    // Attempt to resolve relative to the manifest URL if it's also a file URL
                    // This part might need more robust relative URL resolution logic
                    let base_url = url::Url::parse(manifest_url_for_context).map_err(|e| {
                        AppError::Service(format!(
                            "Invalid base URL for relative bundle path: {}",
                            e
                        ))
                    })?;
                    let joined_url = base_url.join(path_str).map_err(|e| {
                        AppError::Service(format!("Failed to join URL for bundle: {}", e))
                    })?;
                    PathBuf::from(
                        joined_url
                            .path()
                            .strip_prefix('/')
                            .unwrap_or_else(|| joined_url.path()),
                    )
                }
            };

            debug!(
                "Loading worker executable from: {}",
                absolute_path.display()
            );
            tokio::fs::read(&absolute_path)
                .await
                .map_err(AppError::Io)?
        } else if bundle_pointer.source.scheme().starts_with("http") {
            // Fetch from HTTP/HTTPS
            let response = reqwest::get(bundle_pointer.source.clone())
                .await
                .map_err(|e| {
                    AppError::Network(format!(
                        "Failed to fetch worker bundle from {}: {}",
                        bundle_url_str, e
                    ))
                })?;
            if !response.status().is_success() {
                return Err(AppError::Network(format!(
                    "Failed to fetch worker bundle from {}: Status {}",
                    bundle_url_str,
                    response.status()
                )));
            }
            response
                .bytes()
                .await
                .map_err(|e| {
                    AppError::Network(format!(
                        "Failed to read bundle bytes from {}: {}",
                        bundle_url_str, e
                    ))
                })?
                .to_vec()
        } else {
            // TODO: Support data URLs, IPFS, etc.
            return Err(AppError::Service(format!(
                "Unsupported bundle source scheme: {}",
                bundle_pointer.source.scheme()
            )));
        };

        // TODO: Validate hash if bundle_pointer.hash is Some
        // let calculated_hash = sha512_hash(&executable_bytes);
        // if let Some(expected_hash) = &bundle_pointer.hash {
        //    if &calculated_hash != expected_hash { return Err(...) }
        // }

        let payload = WorkerBundlePayload {
            vm: "nxcc/workerd".to_string(), // TODO: Get from manifest or bundle itself? For now, assume workerd for policies.
            executable: executable_bytes,
            metadata: HashMap::new(), // TODO: Populate if bundle format includes it
        };

        Ok(WorkerBundle::new_from_payload(&payload))
    }

    fn get_cache_filepath(&self, secret_id: &SecretId) -> Option<PathBuf> {
        self.disk_cache_path.as_ref().map(|base_path| {
            let mut hasher = DefaultHasher::new();
            secret_id.hash(&mut hasher);
            let filename = format!("{:x}.policy", hasher.finish());
            base_path.join(filename)
        })
    }

    async fn load_from_disk(
        &self,
        secret_id: &SecretId,
    ) -> Result<Option<FullPolicyPackage>, AppError> {
        if let Some(filepath) = self.get_cache_filepath(secret_id) {
            if !filepath.exists() {
                return Ok(None);
            }

            match tokio::fs::read(&filepath).await {
                Ok(data) => {
                    match ciborium::from_reader::<FullPolicyPackage, _>(&data[..]) {
                        Ok(package) => {
                            // TODO: Add cache expiry check?
                            Ok(Some(package))
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
        package: &FullPolicyPackage,
    ) -> Result<(), AppError> {
        if let Some(filepath) = self.get_cache_filepath(secret_id) {
            // Using Vec as a temporary buffer for serialization
            let mut data = Vec::new();
            match ciborium::into_writer(package, &mut data) {
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
