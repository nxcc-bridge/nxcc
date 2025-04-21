// #![allow(clippy::module_inception)] // Allow module name same as file

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
/// * `trusted_config` - Platform-provided secrets (JWK strings).
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

    // --- Define the Worker ---
    {
        let mut worker_def_list = config_builder.reborrow().init_services(1);
        let mut service_builder = worker_def_list.reborrow().get(0);
        service_builder.set_name(service_name);

        let mut worker_builder = service_builder.init_worker();

        // Set compatibility date (Hardcoded for now, could be configurable/detected)
        worker_builder.set_compatibility_date("2025-04-18");

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

        // 2. Secret Key Bindings (assuming JWK format in bytes)
        for (i, key_bytes) in trusted_config.crypto_keys.iter().enumerate() {
            let key_jwk_str = std::str::from_utf8(key_bytes).map_err(|e| {
                WorkerdVmError::SecretKeyParseFailed(format!("Key {} is not valid UTF-8: {}", i, e))
            })?;

            // Basic validation: Check if it looks like JSON
            if !(key_jwk_str.trim_start().starts_with('{') && key_jwk_str.trim_end().ends_with('}'))
            {
                return Err(WorkerdVmError::SecretKeyParseFailed(format!(
                    "Key {} does not appear to be valid JSON JWK",
                    i
                )));
            }

            // TODO: Add more robust JWK validation if necessary.
            // TODO: Determine key usages from manifest/platform if possible, default to common ones.
            let usages = vec![
                worker::binding::crypto_key::Usage::Sign,
                worker::binding::crypto_key::Usage::Verify,
                worker::binding::crypto_key::Usage::Encrypt,
                worker::binding::crypto_key::Usage::Decrypt,
            ]; // Example default usages

            bindings.push((
                format!("SECRET_KEY_{}", i),
                BindingType::CryptoKey {
                    jwk: key_jwk_str.to_string(),
                    usages,
                    extractable: false, // Default to non-extractable for security
                },
            ));
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
                    BindingType::CryptoKey {
                        jwk,
                        usages,
                        extractable,
                    } => {
                        let mut crypto_key_builder = binding_builder.init_crypto_key();
                        crypto_key_builder.set_jwk(jwk);
                        crypto_key_builder.set_extractable(*extractable);
                        let mut usage_list = crypto_key_builder
                            .reborrow()
                            .init_usages(usages.len() as u32);
                        for (j, usage) in usages.iter().enumerate() {
                            usage_list.set(j as u32, *usage);
                        }
                        // Algorithm name/json could be derived from JWK if needed, but workerd often infers it.
                        // Setting a default or leaving it empty might be okay.
                        crypto_key_builder.init_algorithm().set_name("auto"); // Let workerd infer
                    }
                }
            }
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
    CryptoKey {
        jwk: String,
        usages: Vec<worker::binding::crypto_key::Usage>,
        extractable: bool,
    },
    // Add other types like Text, Data, Service if needed later
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
    use std::path::PathBuf;

    use capnp::schema_capnp::brand::binding;
    use nxcc_interface::proto::vm::{Limits, TrustedConfig, UntrustedConfig};
    use tempfile::tempdir;

    use super::*;

    fn create_mock_configs() -> (UntrustedConfig, TrustedConfig) {
        let untrusted = UntrustedConfig {
            userdata_json: r#"{"userKey": "userDataValue"}"#.to_string(),
            advanced_vm_config: Default::default(),
        };
        let trusted = TrustedConfig {
            crypto_keys: vec![
                r#"{"kty":"oct", "k":"AAECAwQFBgcICQoLDA0ODxAREhM="}"#
                    .as_bytes()
                    .to_vec(), // Example symmetric key JWK
            ],
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

        // Try to deserialize and check some values (optional, requires more effort)
        let message_reader =
            capnp::serialize::read_message(&mut config_bytes.as_slice(), Default::default())?;
        let config_reader: config::Reader = message_reader.get_root()?;

        assert_eq!(config_reader.get_services()?.len(), 1);
        let service = config_reader.get_services()?.get(0);
        assert_eq!(service.get_name()?, "main_svc");
        assert!(service.has_worker());
        let service::Worker(Ok(worker)) = service.which()? else {
            panic!("contained wrong service type");
        };
        assert_eq!(worker.get_compatibility_date()?, "2025-04-18");
        assert!(worker.has_modules());
        let worker::Modules(Ok(modules)) = worker.which()? else {
            panic!("contained no modules");
        };
        let module = modules.get(0);
        assert_eq!(module.get_name()?, "worker.js");
        assert!(module.has_es_module());

        assert_eq!(worker.get_bindings()?.len(), 2); // USER_CONFIG + SECRET_KEY_0
        let binding0 = worker.get_bindings()?.get(0);
        assert_eq!(binding0.get_name()?, "USER_CONFIG");
        assert!(binding0.has_json());
        let worker::binding::Json(Ok(user_config_json)) = binding0.which()? else {
            panic!("missing json binding");
        };
        assert_eq!(user_config_json, untrusted.userdata_json);

        let binding1 = worker.get_bindings()?.get(1);
        assert_eq!(binding1.get_name()?, "SECRET_KEY_0");
        assert!(binding1.has_crypto_key());
        let worker::binding::CryptoKey(Ok(crypto_key)) = binding1.which()? else {
            panic!("missing crypto key binding");
        };
        assert!(crypto_key.has_jwk());
        let worker::binding::crypto_key::Jwk(Ok(jwk)) = crypto_key.which()? else {
            panic!("contained wrong binding crypto key type");
        };
        assert_eq!(jwk, std::str::from_utf8(&trusted.crypto_keys[0])?);

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
        let code = b"from js import Response\ndef on_fetch(req, env):\n  return Response.new(env.SECRET_KEY_0)";
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
        let service = config_reader.get_services()?.get(0);
        assert!(service.has_worker());
        let service::Worker(Ok(worker)) = service.which()? else {
            panic!("contained wrong service type");
        };
        let worker::Modules(Ok(modules)) = worker.which()? else {
            panic!("contained no modules");
        };
        let module = modules.get(0);
        assert_eq!(module.get_name()?, "worker.py");
        assert!(module.has_python_module());
        assert_eq!(worker.get_compatibility_flags()?.len(), 2);
        assert_eq!(worker.get_compatibility_flags()?.get(0)?, "python_workers");

        Ok(())
    }
}
