use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use alloy_primitives::hex;
use nxcc_interface::types::{
    DSSE_WORKER_BUNDLE_PAYLOAD_TYPE, DsseEnvelope, DsseSignatureEntry, FullPolicyPackage, SecretId,
    WorkerBundle, WorkerBundlePayload, WorkerBundlePointer, WorkerManifest,
};
use percent_encoding::percent_decode_str;
use tracing::{debug, error, info, trace, warn};

use crate::{config::Config, error::AppError, web3::gateways::GatewayManager};

#[derive(Clone)]
pub struct PolicyManager {
    gateway_manager: Arc<GatewayManager>,
}

impl PolicyManager {
    pub async fn new(
        gateway_manager: Arc<GatewayManager>,
        _config: &Config,
    ) -> Result<Self, AppError> {
        info!("Policy caching disabled for development");

        Ok(Self { gateway_manager })
    }

    /// Fetches a policy, which consists of a `WorkerManifest` and its corresponding `WorkerBundle`.
    /// The `WorkerManifest` (the policy itself) is validated to ensure it requests no identities.
    pub async fn get_policy(&self, secret_id: &SecretId) -> Result<FullPolicyPackage, AppError> {
        // Policy caching disabled - always fetch fresh policy

        // 3. Fetch manifest from network (this is the "policy")
        let manifest_url = self
            .gateway_manager
            .get_policy_url(
                &secret_id.chain,
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
            .fetch_worker_bundle(&manifest.bundle, &manifest_url)
            .await?;

        let package = FullPolicyPackage { manifest, bundle };

        info!(
            "Successfully fetched and validated policy for {:?}",
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

        // Handle data URLs directly embedded in the manifest URL
        if manifest_url.starts_with("data:") {
            let bytes = decode_data_url(manifest_url)?;
            let manifest: WorkerManifest = serde_json::from_slice(&bytes).map_err(|e| {
                AppError::Service(format!(
                    "Failed to parse worker manifest JSON from data URL: {}",
                    e
                ))
            })?;
            return Ok(manifest);
        }

        // Handle mock URLs for testing/dev
        if manifest_url.starts_with("mock://") {
            warn!(
                "Using local mock worker manifest for policy {:?}",
                secret_id_for_log
            );
            // Load from a fixed local path relative to the Cargo manifest dir
            let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| {
                // When CARGO_MANIFEST_DIR is not set (e.g., when running built binary),
                // try to find the tests directory relative to the current executable
                std::env::current_exe()
                    .ok()
                    .and_then(|exe_path| {
                        // Look for node directory in the path hierarchy
                        exe_path
                            .ancestors()
                            .find(|ancestor| {
                                ancestor.file_name() == Some(std::ffi::OsStr::new("node"))
                                    && ancestor.join("tests/policy/mock_policy.json").exists()
                            })
                            .map(|p| p.to_path_buf())
                    })
                    .map(|node_dir| node_dir.to_string_lossy().to_string())
                    .unwrap_or_else(|| ".".to_string())
            });

            // Handle case where CARGO_MANIFEST_DIR points to daemon subdirectory during tests
            let base_path = Path::new(&manifest_dir);
            let mock_manifest_path =
                if base_path.file_name() == Some(std::ffi::OsStr::new("daemon")) {
                    // If we're in the daemon directory, go up one level to find the tests directory
                    base_path
                        .parent()
                        .unwrap()
                        .join("tests/policy/mock_policy.json")
                } else {
                    // If we're in the root node directory, use the direct path
                    base_path.join("tests/policy/mock_policy.json")
                };

            debug!(
                "Loading mock worker manifest from: {}",
                mock_manifest_path.display()
            );

            let manifest_content = tokio::fs::read_to_string(&mock_manifest_path)
                .await
                .map_err(|e| {
                    AppError::Io(std::io::Error::new(
                        e.kind(),
                        format!(
                            "Failed to read mock policy file '{}': {}",
                            mock_manifest_path.display(),
                            e
                        ),
                    ))
                })?;

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

    pub(crate) async fn fetch_worker_bundle(
        &self,
        bundle_pointer: &WorkerBundlePointer,
        manifest_url_for_context: &str, // Used to resolve relative file URLs for mocks
    ) -> Result<WorkerBundle, AppError> {
        let bundle_url_str = bundle_pointer.source.as_str();
        info!(
            "Fetching worker bundle from URL: {}",
            &bundle_url_str[0..20]
        );

        let dsse_envelope_bytes = if bundle_pointer.source.scheme() == "file" {
            // Handle local file paths, potentially relative for mock scenarios
            let path_str = bundle_pointer.source.path();
            let path = PathBuf::from(path_str.strip_prefix('/').unwrap_or(path_str)); // Handle absolute file paths
            let current_dir = std::env::current_dir().map_err(|e| {
                AppError::Internal(format!("Failed to get current directory: {}", e))
            })?;

            let absolute_path = if path.is_absolute() {
                path
            } else {
                // For relative paths in mock, resolve against CARGO_MANIFEST_DIR or manifest_url context
                // Assuming mock_policy.json specifies relative paths like "tests/policy/mock_worker.js"
                // and manifest_url_for_context is something like "mock://..."
                if manifest_url_for_context.starts_with("mock://") {
                    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| {
                        // When CARGO_MANIFEST_DIR is not set (e.g., when running built binary),
                        // try to find the tests directory relative to the current executable
                        std::env::current_exe()
                            .ok()
                            .and_then(|exe_path| {
                                // Look for node directory in the path hierarchy
                                exe_path
                                    .ancestors()
                                    .find(|ancestor| {
                                        ancestor.file_name() == Some(std::ffi::OsStr::new("node"))
                                            && ancestor.join("tests").exists()
                                    })
                                    .map(|p| p.to_path_buf())
                            })
                            .map(|node_dir| node_dir.to_string_lossy().to_string())
                            .unwrap_or_else(|| ".".to_string())
                    });
                    // Handle case where CARGO_MANIFEST_DIR points to daemon subdirectory during tests
                    let base_path = Path::new(&manifest_dir);
                    if base_path.file_name() == Some(std::ffi::OsStr::new("daemon")) {
                        // If we're in the daemon directory, go up one level to find the tests directory
                        base_path.parent().unwrap().join(path)
                    } else {
                        // If we're in the root node directory, use the direct path
                        base_path.join(path)
                    }
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
            let file_content_bytes = tokio::fs::read(&absolute_path).await.map_err(|e| {
                AppError::Io(std::io::Error::new(
                    e.kind(),
                    format!(
                        "Failed to read worker bundle file '{}': {}",
                        absolute_path.display(),
                        e
                    ),
                ))
            })?;

            // If it's a local file (likely for mocking/testing), and it looks like raw executable (e.g. .js)
            // wrap it in a mock DSSE envelope.
            // In a production scenario, file:// URLs should point to complete DSSE envelopes.
            if absolute_path
                .extension()
                .map_or(false, |ext| ext == "js" || ext == "wasm")
            {
                warn!(
                    "Local file bundle source {} appears to be raw executable; wrapping in mock \
                     DSSE envelope",
                    absolute_path.display(),
                );

                let payload_struct = WorkerBundlePayload {
                    vm: "nxcc/workerd".to_string(), // TODO: Get from manifest or bundle itself?
                    executable: file_content_bytes,
                    metadata: HashMap::new(),
                };
                let json_payload_bytes = serde_json::to_vec(&payload_struct).unwrap();

                let dsse_envelope = DsseEnvelope {
                    payload: base64::encode(&json_payload_bytes),
                    payload_type: DSSE_WORKER_BUNDLE_PAYLOAD_TYPE.to_string(),
                    signatures: vec![DsseSignatureEntry {
                        key_id: Some("mock_policy_key_id".to_string()),
                        sig: base64::encode(
                            b"mock_policy_signature_bytes_longer_than_32_for_base64",
                        ),
                    }],
                };
                serde_json::to_vec(&dsse_envelope).unwrap()
            } else {
                file_content_bytes // Assume it's already a DSSE envelope
            }
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
        } else if bundle_pointer.source.scheme() == "data" {
            decode_data_url(bundle_url_str)?
        } else {
            // TODO: Support IPFS, etc.
            return Err(AppError::Service(format!(
                "Unsupported bundle source scheme: {}",
                bundle_pointer.source.scheme()
            )));
        };

        // Validate hash of the fetched DSSE envelope itself, if provided in the pointer
        if let Some(expected_hash_bytes) = &bundle_pointer.hash {
            use sha2::{Digest, Sha512};
            let calculated_hash_bytes = Sha512::digest(&dsse_envelope_bytes).to_vec();
            if &calculated_hash_bytes != expected_hash_bytes {
                return Err(AppError::Service(format!(
                    "WorkerBundle (DSSE envelope) hash mismatch for {}. Expected {}, got {}",
                    bundle_url_str,
                    hex::encode(expected_hash_bytes),
                    hex::encode(calculated_hash_bytes)
                )));
            }
            debug!(
                "WorkerBundle (DSSE envelope) hash verified for {}",
                bundle_url_str
            );
        }

        let bundle = WorkerBundle(dsse_envelope_bytes);
        // Perform a quick validation that it's a parseable DSSE envelope
        // and that its payloadType is correct.
        bundle
            .payload()
            .map_err(|e| AppError::Validation(format!("WorkerBundle payload is invalid: {}", e)))?;

        Ok(bundle)
    }
}

fn decode_data_url(url: &str) -> Result<Vec<u8>, AppError> {
    let without_scheme = url
        .strip_prefix("data:")
        .ok_or_else(|| AppError::Service(format!("Invalid data URL: {}", url)))?;
    let (meta, data) = without_scheme
        .split_once(',')
        .ok_or_else(|| AppError::Service(format!("Invalid data URL: {}", url)))?;

    if meta.ends_with(";base64") {
        base64::decode(data).map_err(|e| {
            AppError::Service(format!("Failed to decode base64 data URL {}: {}", url, e))
        })
    } else {
        Ok(percent_decode_str(data).collect::<Vec<u8>>())
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, U256};

    use super::*;

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn test_fetch_worker_bundle_from_data_url() {
        let gateway = GatewayManager::new();
        let config = Config::default();
        let pm = PolicyManager::new(Arc::new(gateway), &config)
            .await
            .unwrap();

        let payload_struct = WorkerBundlePayload {
            vm: "test-vm".to_string(),
            executable: b"console.log('hi');".to_vec(),
            metadata: HashMap::new(),
        };
        let json_payload = serde_json::to_vec(&payload_struct).unwrap();
        let dsse = DsseEnvelope {
            payload: base64::encode(&json_payload),
            payload_type: DSSE_WORKER_BUNDLE_PAYLOAD_TYPE.to_string(),
            signatures: vec![DsseSignatureEntry {
                key_id: Some("test".to_string()),
                sig: base64::encode(b"sig"),
            }],
        };
        let dsse_bytes = serde_json::to_vec(&dsse).unwrap();
        let data_url = format!(
            "data:application/json;base64,{}",
            base64::encode(&dsse_bytes)
        );
        let pointer = WorkerBundlePointer {
            source: data_url.parse().unwrap(),
            hash: None,
        };

        let bundle = pm
            .fetch_worker_bundle(&pointer, "mock://manifest")
            .await
            .expect("bundle fetch");
        assert_eq!(bundle.0, dsse_bytes);
    }
}
