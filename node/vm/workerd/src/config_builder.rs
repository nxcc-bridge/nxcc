use std::path::Path;

use capnp::message;
use nxcc_interface::proto::vm::{TrustedConfig, UntrustedConfig};

use crate::{errors::WorkerdVmError, workerd_capnp::*};

/// Represents the type of user code provided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeType {
    EsModule,
    Python,
}

/// Builds the Cap'n Proto configuration for a workerd instance.
///
/// # Arguments
/// * `worker_code` - The raw bytes of the worker script.
/// * `code_type` - The detected type of the worker code (ESM or Python).
/// * `untrusted_config` - User-provided configuration (JSON).
/// * `trusted_config` - Platform-provided secrets (raw bytes).
/// * `uds_path` - The path for the Unix Domain Socket the worker should listen on.
/// * `service_name` - The name for the service entry pointing to the worker (e.g., "main").
/// * `socket_name` - The name for the UDS socket entry (e.g., "uds_invoke").
///
/// # Returns
/// A `Vec<u8>` containing the serialized binary Cap'n Proto configuration.
pub fn build_config(
    worker_code: &[u8],
    code_type: CodeType,
    untrusted_config: &UntrustedConfig,
    trusted_config: &TrustedConfig,
    uds_path: &Path,
    service_name: &str,
    socket_name: &str,
) -> Result<Vec<u8>, WorkerdVmError> {
    let mut message = message::Builder::new_default();
    let mut config_builder = message.init_root::<config::Builder>();

    // --- Define Services (Worker + Internet) ---
    {
        let mut service_list = config_builder.reborrow().init_services(2);

        // First service: the worker
        {
            let mut service_builder = service_list.reborrow().get(0);
            service_builder.set_name(service_name);

            let mut worker_builder = service_builder.init_worker();

            // Set compatibility date (Hardcoded for now, could be configurable/detected)
            worker_builder.set_compatibility_date("2025-04-18");

            // Configure global outbound to use internet service for fetch() calls
            let mut global_outbound = worker_builder.reborrow().init_global_outbound();
            global_outbound.set_name("internet");

            // Add modules based on code type
            let mut modules_list = worker_builder.reborrow().init_modules(1);
            let mut module_builder = modules_list.reborrow().get(0);

            let code_str = std::str::from_utf8(worker_code).map_err(|e| {
                WorkerdVmError::InvalidConfig(format!("Worker code is not valid UTF-8: {}", e))
            })?;

            match code_type {
                CodeType::EsModule => {
                    module_builder.set_name("worker.js"); // Default name
                    module_builder.set_es_module(code_str);
                }
                CodeType::Python => {
                    module_builder.set_name("worker.py"); // Default name
                    module_builder.set_python_module(code_str);
                    // Enable Python compatibility flags
                    let mut flags = worker_builder.reborrow().init_compatibility_flags(2);
                    flags.set(0, "python_workers");
                    // Use a recent/relevant python flag version if known, otherwise omit or use a default
                    flags.set(1, "python_workers_20250116"); // Example flag
                }
            }

            // Add bindings
            let mut bindings = Vec::new();

            // 1. JSON User Config Binding
            if !untrusted_config.userdata_json.is_empty() {
                bindings.push((
                    "USER_CONFIG".to_string(),
                    BindingType::Json(untrusted_config.userdata_json.clone()),
                ));
            }
            // TODO: Handle advanced_vm_config if needed, maybe as separate JSON bindings?

            // 2. Secret Key Bindings (raw binary format) using provided names
            for (name, key_bytes) in trusted_config.secrets.iter() {
                // For secret keys, we use the raw data directly and limit the usages
                // to deriveKey and deriveBits for increased security
                bindings.push((name.clone(), BindingType::Secret(key_bytes.clone())));
            }

            // Add bindings to the worker config
            if !bindings.is_empty() {
                let mut binding_list_builder = worker_builder.init_bindings(bindings.len() as u32);
                for (i, (name, binding_type)) in bindings.iter().enumerate() {
                    let mut binding_builder = binding_list_builder.reborrow().get(i as u32);
                    binding_builder.set_name(name);
                    match binding_type {
                        BindingType::Json(json_str) => {
                            binding_builder.set_json(json_str);
                        }
                        BindingType::Secret(raw_key) => {
                            let mut crypto_key_builder = binding_builder.init_crypto_key();
                            crypto_key_builder.set_raw(raw_key.as_slice());
                            crypto_key_builder.set_extractable(false); // Explicitly not extractable for security

                            let mut usage_list = crypto_key_builder.reborrow().init_usages(2); // Only deriveKey and deriveBits
                            usage_list.set(0, worker::binding::crypto_key::Usage::DeriveKey);
                            usage_list.set(1, worker::binding::crypto_key::Usage::DeriveBits);

                            // Set a generic algorithm that's compatible with key derivation
                            crypto_key_builder.init_algorithm().set_name("HKDF");
                        }
                    }
                }
            }
        }

        // Second service: internet access
        {
            let mut internet_service_builder = service_list.reborrow().get(1);
            internet_service_builder.set_name("internet");

            let mut network_builder = internet_service_builder.init_network();

            // Allow public internet access only (prevents SSRF attacks)
            let mut allow_list = network_builder
                .reborrow()
                .init_allow(if cfg!(debug_assertions) { 2 } else { 1 });
            allow_list.set(0, "public");
            #[cfg(debug_assertions)]
            allow_list.set(1, "local");

            // Configure TLS to trust browser certificate authorities
            let mut tls_options = network_builder.reborrow().init_tls_options();
            tls_options.set_trust_browser_cas(true);
        }
    }

    // --- Define the Socket ---
    {
        let mut socket_list_builder = config_builder.init_sockets(1);
        let mut socket_builder = socket_list_builder.reborrow().get(0);
        socket_builder.set_name(socket_name);
        let uds_addr = format!("unix:{}", uds_path.display());
        socket_builder.set_address(&uds_addr);
        // Configure the socket to serve HTTP requests to the main service
        socket_builder.reborrow().init_http(); // Default HTTP options are likely fine
        let mut service_designator = socket_builder.init_service();
        service_designator.set_name(service_name);
    }

    // Serialize the message
    let mut buffer = Vec::new();
    capnp::serialize::write_message(&mut buffer, &message)?;

    Ok(buffer)
}

/// Helper enum for different binding types during construction.
enum BindingType {
    Json(String),
    Secret(Vec<u8>), // Raw binary secret, will be zeroized after use
}

/// Detects the code type based on simple heuristics.
/// TODO: robustify
pub fn detect_code_type(code: &[u8]) -> Result<CodeType, WorkerdVmError> {
    // Very basic detection: look for Python keywords or common JS patterns.
    // A more robust approach might involve trying to parse or using file extensions if available.
    let code_str = std::str::from_utf8(code).map_err(|_| WorkerdVmError::UnsupportedCodeType)?; // Code must be UTF-8

    // Simple Python check
    if code_str.contains("def ")
        && code_str.contains("import ")
        && !code_str.contains("export default")
        && !code_str.contains("addEventListener")
    {
        // More likely Python
        // Check for `async def on_fetch` or similar patterns expected by workerd python
        if code_str.contains("on_fetch") {
            return Ok(CodeType::Python);
        }
    }

    // Simple JS check (ESM or Service Worker)
    if code_str.contains("export default")
        || code_str.contains("addEventListener")
        || code_str.contains("import ")
    {
        return Ok(CodeType::EsModule); // Treat both SW and ESM as needing esModule for now
    }

    // Fallback or error
    // Let's default to ESM if unsure, but log a warning or return error if confidence is low.
    tracing::warn!("Could not confidently detect code type, defaulting to EsModule.");
    Ok(CodeType::EsModule)
    // Err(WorkerdVmError::UnsupportedCodeType)
}

#[cfg(test)]
mod tests {
    use nxcc_interface::proto::vm::{Limits, TrustedConfig, UntrustedConfig};
    use tempfile::tempdir;

    use super::*;

    fn create_mock_configs() -> (UntrustedConfig, TrustedConfig) {
        let untrusted = UntrustedConfig {
            userdata_json: r#"{"userKey": "userDataValue"}"#.to_string(),
            advanced_vm_config: Default::default(),
        };

        // Using raw bytes for the secret key instead of a JWK
        let mut secrets = std::collections::HashMap::new();
        secrets.insert(
            "MY_SECRET".to_string(),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
        );
        let trusted = TrustedConfig {
            secrets,
            limits: Some(Limits {
                memory_mb: 128,
                cpu_count: 1,
                max_runtime_seconds: 30,
            }),
        };
        (untrusted, trusted)
    }

    #[test]
    fn test_detect_code_type() {
        let js_code = b"export default { fetch() { return new Response('Hello'); } };";
        let py_code =
            b"from js import Response\ndef on_fetch(req, env):\n  return Response.new('Hello')";
        let invalid_code = b"\x80\x90\xA0"; // Invalid UTF-8

        assert_eq!(detect_code_type(js_code).unwrap(), CodeType::EsModule);
        assert_eq!(detect_code_type(py_code).unwrap(), CodeType::Python);
        assert!(matches!(
            detect_code_type(invalid_code),
            Err(WorkerdVmError::UnsupportedCodeType)
        ));
    }

    #[test]
    fn test_build_config_esm() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempdir()?;
        let uds_path = dir.path().join("test.sock");
        let code = b"export default { fetch(req, env) { return new Response(env.USER_CONFIG); } };";
        let (untrusted, trusted) = create_mock_configs();

        let config_bytes = build_config(
            code,
            CodeType::EsModule,
            &untrusted,
            &trusted,
            &uds_path,
            "main_svc",
            "uds_sock",
        )?;

        // Basic validation: check if it's non-empty
        assert!(!config_bytes.is_empty());

        // Try to deserialize and check some values
        let message_reader =
            capnp::serialize::read_message(&mut config_bytes.as_slice(), Default::default())?;
        let config_reader: config::Reader = message_reader.get_root()?;

        // Should now have 2 services: worker + internet
        assert_eq!(config_reader.get_services()?.len(), 2);

        let service = config_reader.get_services()?.get(0);
        assert_eq!(service.get_name()?, "main_svc");
        assert!(service.has_worker());

        let service::Worker(Ok(worker)) = service.which()? else {
            panic!("contained wrong service type");
        };
        assert_eq!(worker.get_compatibility_date()?, "2025-04-18");

        // Check that global outbound is configured for internet access
        assert_eq!(worker.get_global_outbound()?.get_name()?, "internet");

        assert!(worker.has_modules());

        let worker::Modules(Ok(modules)) = worker.which()? else {
            panic!("contained no modules");
        };
        let module = modules.get(0);
        assert_eq!(module.get_name()?, "worker.js");
        assert!(module.has_es_module());

        assert_eq!(worker.get_bindings()?.len(), 2); // USER_CONFIG + MY_SECRET

        let binding0 = worker.get_bindings()?.get(0);
        assert_eq!(binding0.get_name()?, "USER_CONFIG");
        assert!(binding0.has_json());

        let worker::binding::Json(Ok(user_config_json)) = binding0.which()? else {
            panic!("missing json binding");
        };
        assert_eq!(user_config_json, untrusted.userdata_json);

        let binding1 = worker.get_bindings()?.get(1);
        assert_eq!(binding1.get_name()?, "MY_SECRET");
        assert!(binding1.has_crypto_key());

        let worker::binding::CryptoKey(Ok(crypto_key)) = binding1.which()? else {
            panic!("missing crypto key binding");
        };
        assert!(crypto_key.has_raw());

        // Check that the key is not extractable
        assert!(!crypto_key.get_extractable());

        // Check usages are limited to derive operations
        assert_eq!(crypto_key.get_usages()?.len(), 2);
        assert_eq!(
            crypto_key.get_usages()?.get(0)?,
            worker::binding::crypto_key::Usage::DeriveKey
        );
        assert_eq!(
            crypto_key.get_usages()?.get(1)?,
            worker::binding::crypto_key::Usage::DeriveBits
        );

        // Check internet service configuration
        let internet_service = config_reader.get_services()?.get(1);
        assert_eq!(internet_service.get_name()?, "internet");
        assert!(internet_service.has_network());

        let service::Network(Ok(network)) = internet_service.which()? else {
            panic!("internet service should be a network service");
        };
        assert_eq!(network.get_allow()?.len(), 2);
        assert_eq!(network.get_allow()?.get(0)?, "public");
        assert!(network.has_tls_options());
        assert!(network.get_tls_options()?.get_trust_browser_cas());

        // Check socket configuration
        assert_eq!(config_reader.get_sockets()?.len(), 1);
        let socket = config_reader.get_sockets()?.get(0);
        assert_eq!(socket.get_name()?, "uds_sock");
        assert!(socket.get_address()?.to_str()?.contains("test.sock"));
        assert!(socket.has_http());
        assert_eq!(socket.get_service()?.get_name()?, "main_svc");

        Ok(())
    }

    #[test]
    fn test_build_config_python() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempdir()?;
        let uds_path = dir.path().join("py_test.sock");
        let code = b"from js import Response\ndef on_fetch(req, env):\n  return Response.new(env.MY_SECRET)";
        let (untrusted, trusted) = create_mock_configs();

        let config_bytes = build_config(
            code,
            CodeType::Python,
            &untrusted,
            &trusted,
            &uds_path,
            "py_svc",
            "py_sock",
        )?;

        assert!(!config_bytes.is_empty());

        let message_reader =
            capnp::serialize::read_message(&mut config_bytes.as_slice(), Default::default())?;
        let config_reader: config::Reader = message_reader.get_root()?;

        // Should have 2 services: worker + internet
        assert_eq!(config_reader.get_services()?.len(), 2);

        let service = config_reader.get_services()?.get(0);
        assert!(service.has_worker());

        let service::Worker(Ok(worker)) = service.which()? else {
            panic!("contained wrong service type");
        };

        // Check that global outbound is configured for internet access
        assert_eq!(worker.get_global_outbound()?.get_name()?, "internet");

        let worker::Modules(Ok(modules)) = worker.which()? else {
            panic!("contained no modules");
        };

        let module = modules.get(0);
        assert_eq!(module.get_name()?, "worker.py");
        assert!(module.has_python_module());

        assert_eq!(worker.get_compatibility_flags()?.len(), 2);
        assert_eq!(worker.get_compatibility_flags()?.get(0)?, "python_workers");

        // Verify the crypto key binding for the Python worker
        let binding1 = worker.get_bindings()?.get(1);
        assert_eq!(binding1.get_name()?, "MY_SECRET");

        let worker::binding::CryptoKey(Ok(crypto_key)) = binding1.which()? else {
            panic!("missing crypto key binding");
        };

        // Verify internet service exists
        let internet_service = config_reader.get_services()?.get(1);
        assert_eq!(internet_service.get_name()?, "internet");

        Ok(())
    }
}
