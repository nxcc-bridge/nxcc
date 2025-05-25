use std::{collections::HashMap, error::Error as _};

use nxcc_interface::proto::vm::Limits;
use tokio::time::{self, Duration};

use super::*;

fn create_mock_configs() -> (UntrustedConfig, TrustedConfig) {
    let untrusted = UntrustedConfig {
        userdata_json: r#"{"message": "Hello from config!"}"#.to_string(),
        advanced_vm_config: HashMap::new(),
    };
    let mut secrets = HashMap::new();
    secrets.insert("MY_SECRET".to_string(), vec![0u8; 32]);
    let trusted = TrustedConfig {
        secrets,
        limits: Some(Limits {
            memory_mb: 128,
            cpu_count: 1,
            max_runtime_seconds: 5,
        }),
    };
    (untrusted, trusted)
}

fn create_js_code(id: &str) -> Vec<u8> {
    format!(
        r#"
        export default {{
            async fetch(request, env, ctx) {{
                let body = await request.text();
                return new Response("Response from {}: " + body);
            }}
        }}
        "#,
        id
    )
    .into_bytes()
}

fn create_js_config_code() -> Vec<u8> {
    r#"
    export default {
        async fetch(request, env, ctx) {
            let config = env.USER_CONFIG;
            let key_present = typeof env.MY_SECRET !== 'undefined';
            return new Response(`Config message: ${config.message}, Key bound: ${key_present}`);
        }
    }
    "#
    .to_string()
    .into_bytes()
}

async fn wait_for_status(
    vmm: &WorkerdVmm,
    id: &str,
    target_status: WorkerStatus,
    timeout: Duration,
) -> Result<WorkerStatus, String> {
    let start = time::Instant::now();
    loop {
        match vmm.get_worker_status(id.to_string()).await {
            Ok(status) => {
                if status == target_status {
                    return Ok(status);
                }
                if target_status == WorkerStatus::Stopped && status == WorkerStatus::Error {
                    warn!(
                        "Worker {} reached ERROR state while waiting for STOPPED.",
                        id
                    );
                    return Ok(status);
                }
            }
            Err(e) => {
                if (target_status == WorkerStatus::Stopped || target_status == WorkerStatus::Error)
                    && start.elapsed() > Duration::from_secs(1)
                {
                    if let Some(werr) = e.source().and_then(|s| s.downcast_ref::<WorkerdVmError>())
                    {
                        if matches!(werr, WorkerdVmError::WorkerNotFound(_)) {
                            info!(
                                "Worker {} not found, assuming Stopped/Error state reached.",
                                id
                            );
                            return Ok(target_status);
                        }
                    }
                }
                if start.elapsed() >= timeout {
                    return Err(format!(
                        "Error getting status for {}: {}. Target: {:?}",
                        id, e, target_status
                    ));
                }
            }
        }
        if start.elapsed() >= timeout {
            let final_status = vmm.get_worker_status(id.to_string()).await;
            return Err(format!(
                "Timeout waiting for worker {} to reach {:?}. Last status: {:?}",
                id, target_status, final_status
            ));
        }
        time::sleep(Duration::from_millis(200)).await;
    }
}

#[tokio::test]
#[ignore] // Requires workerd binary on PATH
#[tracing_test::traced_test]
async fn test_start_invoke_stop_single_worker() -> Result<(), Box<dyn std::error::Error>> {
    let vmm = WorkerdVmm::new(Default::default());
    let (untrusted, trusted) = create_mock_configs();
    let code = create_js_code("single");

    let worker_id = vmm
        .start_worker(code, untrusted, trusted)
        .await
        .expect("Failed to start worker");

    let status = wait_for_status(
        &vmm,
        &worker_id,
        WorkerStatus::Running,
        Duration::from_secs(15),
    )
    .await?;
    assert_eq!(status, WorkerStatus::Running);

    let payload = b"test payload".to_vec();
    let response = vmm
        .invoke_worker(worker_id.clone(), "fetch".to_string(), payload)
        .await
        .expect("Failed to invoke worker");
    assert_eq!(
        String::from_utf8_lossy(&response),
        "Response from single: test payload"
    );

    vmm.stop_worker(worker_id.clone())
        .await
        .expect("Failed to stop worker");

    let final_status = wait_for_status(
        &vmm,
        &worker_id,
        WorkerStatus::Stopped,
        Duration::from_secs(5),
    )
    .await?;
    assert!(
        final_status == WorkerStatus::Stopped || final_status == WorkerStatus::Error,
        "Final status was {:?}",
        final_status
    );

    let invoke_stopped_result = vmm
        .invoke_worker(
            worker_id.clone(),
            "fetch".to_string(),
            b"after stop".to_vec(),
        )
        .await;
    assert!(invoke_stopped_result.is_err());
    let err = invoke_stopped_result
        .unwrap_err()
        .source()
        .unwrap()
        .downcast_ref::<WorkerdVmError>()
        .unwrap()
        .to_string();
    assert!(
        err.contains("Worker instance not found")
            || err.contains("Worker is not in a runnable state"),
        "Unexpected error: {}",
        err
    );

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_multiple_workers_lifecycle() -> Result<(), Box<dyn std::error::Error>> {
    let vmm = WorkerdVmm::new(Default::default());
    let (untrusted1, trusted1) = create_mock_configs();
    let (untrusted2, trusted2) = create_mock_configs();
    let code1 = create_js_code("worker1");
    let code2 = create_js_code("worker2");

    let id1 = vmm
        .start_worker(code1, untrusted1, trusted1)
        .await
        .expect("Failed to start worker 1");
    info!("Started worker 1: {}", id1);
    let id2 = vmm
        .start_worker(code2, untrusted2, trusted2)
        .await
        .expect("Failed to start worker 2");
    info!("Started worker 2: {}", id2);

    let status1 =
        wait_for_status(&vmm, &id1, WorkerStatus::Running, Duration::from_secs(15)).await?;
    assert_eq!(status1, WorkerStatus::Running);
    let status2 =
        wait_for_status(&vmm, &id2, WorkerStatus::Running, Duration::from_secs(15)).await?;
    assert_eq!(status2, WorkerStatus::Running);

    let running_workers = vmm.list_running_workers().await?;
    assert!(running_workers.contains(&id1));
    assert!(running_workers.contains(&id2));
    assert_eq!(running_workers.len(), 2);

    let resp1 = vmm
        .invoke_worker(id1.clone(), "fetch".to_string(), b"ping1".to_vec())
        .await?;
    assert_eq!(
        String::from_utf8_lossy(&resp1),
        "Response from worker1: ping1"
    );

    let resp2 = vmm
        .invoke_worker(id2.clone(), "fetch".to_string(), b"ping2".to_vec())
        .await?;
    assert_eq!(
        String::from_utf8_lossy(&resp2),
        "Response from worker2: ping2"
    );

    vmm.stop_worker(id1.clone())
        .await
        .expect("Failed to stop worker 1");

    let status1_after_stop =
        wait_for_status(&vmm, &id1, WorkerStatus::Stopped, Duration::from_secs(5)).await?;
    assert!(
        status1_after_stop == WorkerStatus::Stopped || status1_after_stop == WorkerStatus::Error,
        "Worker 1 final status was {:?}",
        status1_after_stop
    );

    let status2_after_stop = vmm.get_worker_status(id2.clone()).await?;
    assert_eq!(status2_after_stop, WorkerStatus::Running);

    let running_workers_after_stop = vmm.list_running_workers().await?;
    assert!(!running_workers_after_stop.contains(&id1));
    assert!(running_workers_after_stop.contains(&id2));
    assert_eq!(running_workers_after_stop.len(), 1);

    let invoke_stopped_result = vmm
        .invoke_worker(id1.clone(), "fetch".to_string(), b"post-stop".to_vec())
        .await;
    assert!(invoke_stopped_result.is_err());
    let err_str = invoke_stopped_result.unwrap_err().to_string();
    assert!(
        err_str.contains("Worker instance not found")
            || err_str.contains("Worker is not in a runnable state"),
        "Unexpected error string: {}",
        err_str
    );

    let resp2_again = vmm
        .invoke_worker(id2.clone(), "fetch".to_string(), b"ping2 again".to_vec())
        .await?;
    assert_eq!(
        String::from_utf8_lossy(&resp2_again),
        "Response from worker2: ping2 again"
    );

    vmm.stop_worker(id2.clone())
        .await
        .expect("Failed to stop worker 2");
    let status2_final =
        wait_for_status(&vmm, &id2, WorkerStatus::Stopped, Duration::from_secs(5)).await?;
    assert!(
        status2_final == WorkerStatus::Stopped || status2_final == WorkerStatus::Error,
        "Worker 2 final status was {:?}",
        status2_final
    );

    let running_workers_final = vmm.list_running_workers().await?;
    assert!(running_workers_final.is_empty());

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_worker_config_bindings() -> Result<(), Box<dyn std::error::Error>> {
    let vmm = WorkerdVmm::new(Default::default());
    let (untrusted, trusted) = create_mock_configs();
    let code = create_js_config_code();

    let worker_id = vmm
        .start_worker(code, untrusted, trusted)
        .await
        .expect("Failed to start config worker");

    let status = wait_for_status(
        &vmm,
        &worker_id,
        WorkerStatus::Running,
        Duration::from_secs(15),
    )
    .await?;
    assert_eq!(status, WorkerStatus::Running);

    // Instead of a simple expect, handle the error and print status/logs
    let response_result = vmm
        .invoke_worker(worker_id.clone(), "fetch".to_string(), vec![])
        .await;
    if let Err(e) = response_result {
        // Print more info if you like
        let worker_status = vmm.get_worker_status(worker_id.clone()).await?;
        let worker_logs = vmm.get_worker_logs(worker_id.clone()).await?;
        panic!(
            "Failed to invoke config worker: {e}\nWorker status: \
             {worker_status:?}\nLogs:\n{worker_logs}"
        );
    }
    let response = response_result.unwrap();

    assert_eq!(
        String::from_utf8_lossy(&response),
        "Config message: Hello from config!, Key bound: true"
    );

    vmm.stop_worker(worker_id.clone()).await?;
    wait_for_status(
        &vmm,
        &worker_id,
        WorkerStatus::Stopped,
        Duration::from_secs(5),
    )
    .await?;

    Ok(())
}

#[tokio::test]
async fn test_error_handling_non_existent_worker() {
    let vmm = WorkerdVmm::new(Default::default());
    let non_existent_id = "id-does-not-exist".to_string();

    let stop_res = vmm.stop_worker(non_existent_id.clone()).await;
    assert!(stop_res.is_err());
    assert!(
        stop_res
            .unwrap_err()
            .to_string()
            .contains("Worker instance not found")
    );

    let invoke_res = vmm
        .invoke_worker(non_existent_id.clone(), "fetch".to_string(), vec![])
        .await;
    assert!(invoke_res.is_err());
    assert!(
        invoke_res
            .unwrap_err()
            .to_string()
            .contains("Worker instance not found")
    );

    let status_res = vmm.get_worker_status(non_existent_id.clone()).await;
    assert!(status_res.is_err());
    assert!(
        status_res
            .unwrap_err()
            .to_string()
            .contains("Worker instance not found")
    );

    let logs_res = vmm.get_worker_logs(non_existent_id.clone()).await;
    assert!(logs_res.is_err());
    assert!(
        logs_res
            .unwrap_err()
            .to_string()
            .contains("Worker instance not found")
    );
}

#[tokio::test]
async fn test_get_attestation_unsupported() {
    let vmm = WorkerdVmm::new(Default::default());
    let attestation_res = vmm.get_attestation(vec![1, 2, 3]).await;
    assert!(attestation_res.is_err());
    assert!(
        attestation_res
            .unwrap_err()
            .to_string()
            .contains("Attestation not supported")
    );
}

#[tokio::test]
async fn test_start_worker_invalid_code() {
    let vmm = WorkerdVmm::new(Default::default());
    let (untrusted, trusted) = create_mock_configs();
    let invalid_code = vec![0xff, 0xfe, 0xfd];

    let start_res = vmm.start_worker(invalid_code, untrusted, trusted).await;
    assert!(start_res.is_err());
    let err_msg = start_res.unwrap_err().to_string();
    assert!(
        err_msg.contains("Unsupported worker code type") || err_msg.contains("not valid UTF-8")
    );
}

fn derive_hkdf_sha256(key: &[u8], salt: &[u8], info: &[u8]) -> Vec<u8> {
    use hkdf::Hkdf;
    use hmac::Hmac;
    use sha2::Sha256;

    let hk = Hkdf::<Sha256, Hmac<Sha256>>::new(Some(salt), key);
    let mut derived_bits = vec![0u8; 32]; // 256 bits
    hk.expand(info, &mut derived_bits)
        .expect("HKDF expand failed"); // expand returns Result<(), InvalidLength>

    derived_bits
}

fn create_mock_configs_with_multiple_keys() -> (UntrustedConfig, TrustedConfig) {
    let untrusted = UntrustedConfig {
        userdata_json: r#"{"user":"Alice"}"#.to_string(),
        advanced_vm_config: HashMap::new(),
    };
    // For demo, we create two 32-byte keys with different patterns:
    // Key0 = [0x00; 32], Key1 = [0xFF; 32]
    let key0 = vec![0x00; 32];
    let key1 = vec![0xFF; 32];
    let mut secrets = HashMap::new();
    secrets.insert("KEY_A".to_string(), key0);
    secrets.insert("KEY_B".to_string(), key1);
    let trusted = TrustedConfig {
        secrets,
        limits: Some(Limits {
            memory_mb: 128,
            cpu_count: 1,
            max_runtime_seconds: 5,
        }),
    };
    (untrusted, trusted)
}

fn create_js_multi_key_test_code() -> Vec<u8> {
    r#"
    export default {
      async fetch(request, env) {
        // Hard-coded salt/info for the test
        const salt = new Uint8Array([1,2,3,4]);
        const info = new Uint8Array([5,6,7,8]);
        const derivedResults = [];

        // For each named secret in the environment, run HKDF deriveBits
        const secretNames = ['KEY_A', 'KEY_B'];
        for (const keyName of secretNames) {
          if (!(keyName in env)) {
            continue;
          }
          const cryptoKey = env[keyName];

          // Derive 256 bits
          const derivedBits = await crypto.subtle.deriveBits(
            {
              name: 'HKDF',
              hash: 'SHA-256',
              salt,
              info
            },
            cryptoKey,
            256
          );

          // Convert derived bits to base64 for returning
          const derivedArr = new Uint8Array(derivedBits);
          let b64 = '';
          for (let j = 0; j < derivedArr.length; j++) {
            b64 += String.fromCharCode(derivedArr[j]);
          }
          // Simple base64
          const base64Result = btoa(b64);

          derivedResults.push({
            keyName,
            derivedBase64: base64Result
          });
        }

        return new Response(JSON.stringify(derivedResults));
      }
    };
    "#
    .to_string()
    .into_bytes()
}

#[tokio::test]
#[ignore] // Requires workerd binary on PATH
#[tracing_test::traced_test]
async fn test_multiple_secret_keys_derived_bits() -> Result<(), Box<dyn std::error::Error>> {
    use base64::Engine as _;
    // 1) Create the VMM and configs with multiple keys
    let vmm = WorkerdVmm::new(Default::default());
    let (untrusted, trusted) = create_mock_configs_with_multiple_keys();
    // 2) Provide the JS that uses HKDF on each named secret
    let code = create_js_multi_key_test_code();

    // 3) Start the worker
    let worker_id = vmm.start_worker(code, untrusted, trusted).await?;
    // Wait for it to be running
    let status = wait_for_status(
        &vmm,
        &worker_id,
        WorkerStatus::Running,
        Duration::from_secs(15),
    )
    .await?;
    assert_eq!(status, WorkerStatus::Running);

    // 4) Invoke the worker (no special payload needed).
    //    The worker will return a JSON array of {keyName, derivedBase64} objects.
    let invoke_result = vmm
        .invoke_worker(worker_id.clone(), "fetch".to_string(), vec![])
        .await?;
    let response_str = String::from_utf8_lossy(&invoke_result);
    let parsed: serde_json::Value = serde_json::from_str(&response_str)?;

    // 5) Compare the derived bits from the worker with our local HKDF derivation
    //    We used salt=[1,2,3,4] info=[5,6,7,8] in the JS snippet above.
    let salt = [1u8, 2, 3, 4];
    let info = [5u8, 6, 7, 8];

    // We inserted two keys KEY_A=[0x00;32], KEY_B=[0xFF;32]
    let expected_keys = [vec![0x00; 32], vec![0xFF; 32]];

    // Make sure we got an array in the response
    let arr = parsed
        .as_array()
        .ok_or("Expected JSON array in worker response")?;
    assert_eq!(
        arr.len(),
        expected_keys.len(),
        "Number of keys doesn't match"
    );

    for (name, raw_key) in [("KEY_A", &expected_keys[0]), ("KEY_B", &expected_keys[1])].iter() {
        // Re-derive bits in Rust
        let local_derived = derive_hkdf_sha256(raw_key, &salt, &info);

        // Worker response must have an object with "derivedBase64"
        let worker_obj = arr
            .iter()
            .find(|v| v.get("keyName").map(|n| n == *name).unwrap_or(false))
            .ok_or("Missing object for key")?;
        let derived_base64 = worker_obj
            .get("derivedBase64")
            .ok_or("Missing derivedBase64 field")?
            .as_str()
            .ok_or("derivedBase64 is not a string")?;

        // Convert worker's base64 string back to raw bytes
        let worker_derived = base64::prelude::BASE64_STANDARD
            .decode(derived_base64)
            .map_err(|e| format!("Failed to decode base64 from worker: {e}"))?;

        // Compare
        assert_eq!(
            local_derived, worker_derived,
            "Mismatch in derived bits for {name}"
        );
    }

    // 6) Stop worker
    vmm.stop_worker(worker_id.clone()).await?;
    wait_for_status(
        &vmm,
        &worker_id,
        WorkerStatus::Stopped,
        Duration::from_secs(5),
    )
    .await?;

    Ok(())
}

// TODO: Add test for workerd failing to start if a mock workerd script is implemented.
