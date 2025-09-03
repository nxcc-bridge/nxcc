use nxcc_interface::{
    proto::vm::{HttpRequest as ProtoHttpRequest, HttpResponse as ProtoHttpResponse},
    types::{
        attestation::{
            AttestationBundle, InterfaceConfirmationMethod, InterfaceJwk, InterfaceMeasurement,
            StandardizedAttestationClaims,
        },
        policy::{PolicyExecutionContextForWorker, PolicyExecutionReport, PolicyExecutionRequest},
        worker::events::EventPayload,
    },
};
use tracing::{debug, error, info, warn};

use super::{RunnerError, RunnerService, VmEventInvocation};

impl RunnerService {
    /// Executes a policy worker against multiple contexts.
    pub async fn execute_policy(
        &self,
        worker_id: String,
        mut contexts: Vec<PolicyExecutionRequest>,
    ) -> Result<Vec<PolicyExecutionRequest>, RunnerError> {
        info!(
            "Executing policy worker '{}' for {} contexts",
            worker_id,
            contexts.len()
        );

        // First, enhance contexts with attestation claims by verifying their attestation reports
        for context in &mut contexts {
            // Try to verify the attestation and extract standardized claims
            match self
                .attestation_manager
                .verify_and_extract_claims(&context.env_report.attestation)
                .await
            {
                Ok(claims) => {
                    info!(
                        "Successfully verified attestation with platform {}",
                        claims.eat_profile
                    );
                    // Convert from attestation StandardizedClaims to interface StandardizedAttestationClaims
                    let interface_claims = self.convert_claims_to_interface(*claims);
                    context.attestation_claims = Some(interface_claims);
                }
                Err(e) => {
                    warn!(
                        "Failed to verify attestation: {}. Policy will run without verified \
                         claims.",
                        e
                    );
                    // Continue without claims - policy can decide whether to allow this
                }
            }
        }

        let vm_id = {
            let worker_map_guard = self.worker_map.read().await;
            worker_map_guard
                .get(&worker_id)
                .cloned()
                .ok_or_else(|| RunnerError::WorkerNotFound(worker_id.clone()))?
        };

        // Create sanitized contexts for policy worker (exclude system userdata)
        let sanitized_contexts: Vec<PolicyExecutionContextForWorker> = contexts
            .iter()
            .map(|context| {
                // Extract user-provided data from the full attestation
                // The user_data_binding contains the user data that was bound to the attestation
                // We extract the user portion (excluding ephemeral keys and system data)
                let user_provided_data = context.env_report.attestation.detached_userdata.clone();
                context.for_policy_worker(user_provided_data)
            })
            .collect();

        let payload = serde_json::to_vec(&sanitized_contexts).unwrap();

        let mut vms_guard = self.vms.write().await; // Write lock for mutable client
        let client = vms_guard
            .get_mut(&vm_id)
            .ok_or_else(|| RunnerError::VmNotAttached(vm_id.clone()))?;

        const POLICY_HANDLER_NAME: &str = "_policy"; // Default handler for policy execution
        let result_payload = client
            .invoke_worker(worker_id.clone(), POLICY_HANDLER_NAME.to_string(), payload)
            .await
            .map_err(|e| {
                error!(
                    "Policy execution (handler: {}) invocation failed for worker '{}' in VM '{}': \
                     {}",
                    POLICY_HANDLER_NAME, worker_id, vm_id, e
                );
                RunnerError::VmConnection(e) // Or map specific errors
            })?;

        // Deserialize the result payload from the VM
        // For policy execution, the VM returns Vec<bool> indicating success for each context index.
        // The handler_name for policy execution is fixed (e.g., "_policy").
        let results: Vec<bool> = serde_json::from_slice(result_payload.as_slice())
            .map_err(|e| RunnerError::Deserialization(e.to_string()))?;

        if results.len() != contexts.len() {
            error!(
                "Mismatched number of results ({}) and contexts ({}) from policy worker '{}'",
                results.len(),
                contexts.len(),
                worker_id
            );
            return Err(RunnerError::PolicyExecutionFailed(
                "Mismatched result count".to_string(),
            ));
        }

        let current_time = chrono::Utc::now().timestamp() as u64;
        let mut satisfied_contexts = Vec::new();

        for (i, context) in contexts.into_iter().enumerate() {
            if results[i] {
                // Policy satisfied for this context
                debug!("Policy satisfied for context {}", i);
                let report = PolicyExecutionReport {
                    request: context.clone(),
                    decision: true,
                    timestamp: current_time,
                };
                // Store authorization in the secrets service
                self.secrets.store_authorization(report);
                satisfied_contexts.push(context);
            } else {
                debug!("Policy denied for context {}", i);
            }
        }

        info!(
            "Policy execution complete for worker '{}'. {}/{} contexts satisfied.",
            worker_id,
            satisfied_contexts.len(),
            results.len()
        );
        Ok(satisfied_contexts)
    }

    /// Helper method to convert attestation StandardizedClaims to interface StandardizedAttestationClaims
    fn convert_claims_to_interface(
        &self,
        claims: nxcc_attestation::StandardizedClaims,
    ) -> StandardizedAttestationClaims {
        use nxcc_interface::types::attestation::StandardizedAttestationClaims;

        // Convert attestation Measurement to interface InterfaceMeasurement
        let interface_measurements: Vec<InterfaceMeasurement> = claims
            .measurements
            .into_iter()
            .map(|m| InterfaceMeasurement {
                val: m.val,
                alg: m.alg,
                measurement_type: m.measurement_type,
                vendor: m.vendor,
                version: m.version,
            })
            .collect();

        // Convert confirmation method if present
        let cnf = claims.cnf.map(|cm| match cm {
            nxcc_attestation::types::ConfirmationMethod::Jwk { jwk } => {
                InterfaceConfirmationMethod::Jwk {
                    jwk: InterfaceJwk {
                        kty: jwk.kty,
                        crv: jwk.crv,
                        x: jwk.x,
                        y: jwk.y,
                    },
                }
            }
            nxcc_attestation::types::ConfirmationMethod::CoseKey { cose_key } => {
                InterfaceConfirmationMethod::CoseKey { cose_key }
            }
        });

        // Convert EAT StandardizedClaims to interface StandardizedAttestationClaims
        StandardizedAttestationClaims {
            // EAT-compliant fields (using exact claim names)
            iat: claims.iat,
            eat_nonce: claims.eat_nonce,
            ueid: claims.ueid,
            oemid: claims.oemid,
            hwmodel: claims.hwmodel,
            hwversion: claims.hwversion,
            dbgstat: claims.dbgstat,
            oemboot: claims.oemboot,
            swname: claims.swname,
            swversion: claims.swversion,
            measurements: interface_measurements,
            cnf,
            intuse: claims.intuse,
            uptime: claims.uptime,
            bootcount: claims.bootcount,
            bootseed: claims.bootseed,
            eat_profile: claims.eat_profile,
        }
    }

    /// Delivers a batch of asynchronous events to appropriate workers.
    pub async fn deliver_batch_events(
        &self,
        events: Vec<(String, String, EventPayload<'static>)>, // (worker_id, handler_name, event_payload)
    ) -> Result<(), RunnerError> {
        debug!("Received batch of {} events for delivery.", events.len());
        for (worker_id, handler_name, event_payload) in events {
            // 1. Verification (stub)
            debug!(
                "Stub verification for event to worker_id: {}, handler: {}",
                worker_id, handler_name
            );

            // 2. Serialize VmEventInvocation for VM
            let vm_invocation = VmEventInvocation {
                handler: handler_name.clone(), // Clone handler_name
                event_payload,                 // Move event_payload
            };
            let vm_invocation_payload_bytes = serde_json::to_vec(&vm_invocation).map_err(|e| {
                RunnerError::Internal(format!("Failed to serialize VmEventInvocation: {}", e))
            })?;

            // 3. Send to internal queue
            if let Err(e) = self.event_tx.send((
                worker_id.clone(),
                handler_name.clone(),
                vm_invocation_payload_bytes,
            )) {
                error!(
                    "Failed to send event (handler: {}) to internal queue for worker {}: {}",
                    handler_name, worker_id, e
                );
                return Err(RunnerError::EventSendError(e.to_string()));
            }
        }
        Ok(())
    }

    /// Invokes a worker with an HTTP request.
    pub async fn invoke_http_worker(
        &self,
        worker_id: String,
        http_request: ProtoHttpRequest,
    ) -> Result<ProtoHttpResponse, RunnerError> {
        info!(
            "Invoking HTTP worker '{}' with URI: {}",
            worker_id, http_request.uri
        );

        let vm_id = {
            let worker_map_guard = self.worker_map.read().await;
            worker_map_guard
                .get(&worker_id)
                .cloned()
                .ok_or_else(|| RunnerError::WorkerNotFound(worker_id.clone()))?
        };

        let mut vms_guard = self.vms.write().await;
        let client = vms_guard
            .get_mut(&vm_id)
            .ok_or_else(|| RunnerError::VmNotAttached(vm_id.clone()))?;

        client
            .invoke_http(worker_id.clone(), http_request)
            .await
            .map_err(|e| {
                error!(
                    "HTTP invocation failed for worker '{}' in VM '{}': {}",
                    worker_id, vm_id, e
                );
                RunnerError::VmConnection(e)
            })
    }
}
