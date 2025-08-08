use std::{collections::HashMap, sync::Arc};

use alloy_primitives::Address;
use alloy_provider::{DynProvider, Provider, ProviderBuilder};
use alloy_sol_types::{sol, SolEvent as _};
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use nxcc_interface::{
    proto::daemon::SubmitWorkOrderRequest,
    types::{
        DsseEnvelope, DsseSignatureEntry, Web3Event, WorkOrderPayload, WorkerBundlePayload,
        WorkerBundlePointer, WorkerEvent, WorkerEventKind, WorkerManifest,
    },
};

sol!(
    #[sol(rpc, abi)]
    "../node/tests/contracts/TestEvents.sol"
);

const TEST_EVENTS_JSON: &[u8] =
    include_bytes!("../../node/tests/out/TestEvents.sol/TestEvents.json");

const DSSE_WORKER_BUNDLE_PAYLOAD_TYPE: &str = "application/vnd.nxcc.workerbundlepayload.v1+json";
const DSSE_WORK_ORDER_PAYLOAD_TYPE: &str = "application/vnd.nxcc.workorderpayload.v1+json";

fn create_worker_bundle_dsse_bytes(js_file_path: &str) -> Result<Vec<u8>> {
    let js_content = std::fs::read(js_file_path).with_context(|| js_file_path.to_string())?;
    let payload = WorkerBundlePayload {
        vm: "nxcc/workerd".to_string(),
        executable: js_content,
        metadata: HashMap::new(),
    };
    let payload_json = serde_json::to_vec(&payload)?;
    let payload_b64 = B64.encode(&payload_json);
    let dsse_envelope = DsseEnvelope {
        payload: payload_b64,
        payload_type: DSSE_WORKER_BUNDLE_PAYLOAD_TYPE.to_string(),
        signatures: vec![DsseSignatureEntry {
            key_id: Some("bench-key".to_string()),
            sig: B64.encode("benches"),
        }],
    };
    serde_json::to_vec(&dsse_envelope).context("Failed to serialize worker bundle DSSE envelope")
}

pub fn create_work_order(
    worker_js_path: &str,
    userdata: Option<serde_json::Value>,
    events: Option<Vec<WorkerEvent>>,
) -> Result<DsseEnvelope> {
    let worker_path = format!("./workers/dist/{}", worker_js_path);

    let worker_bundle_bytes = create_worker_bundle_dsse_bytes(&worker_path)?;
    let bundle_b64 = B64.encode(&worker_bundle_bytes);

    let data_url = format!("data:application/json;base64,{}", bundle_b64);

    let bundle_pointer = WorkerBundlePointer {
        source: url::Url::parse(&data_url)?,
        hash: None,
    };

    let manifest = WorkerManifest {
        bundle: bundle_pointer,
        identities: vec![],
        userdata: userdata
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default(),
    };

    let work_order_payload = WorkOrderPayload {
        id: format!("bench-wo-{}", rand::random::<u64>()),
        worker: manifest,
        events: events.unwrap_or_default(),
    };

    let payload_bytes = serde_json::to_vec(&work_order_payload)?;
    let payload_b64 = B64.encode(&payload_bytes);

    let work_order_dsse = DsseEnvelope {
        payload: payload_b64,
        payload_type: DSSE_WORK_ORDER_PAYLOAD_TYPE.to_string(),
        signatures: vec![DsseSignatureEntry {
            key_id: Some("bench-key".to_string()),
            sig: B64.encode("benches"),
        }],
    };

    Ok(work_order_dsse)
}

pub fn create_submit_request(work_order: DsseEnvelope) -> Result<SubmitWorkOrderRequest> {
    let dsse_bytes = serde_json::to_vec(&work_order)?;
    Ok(SubmitWorkOrderRequest {
        work_order_dsse_bytes: dsse_bytes,
    })
}

pub async fn deploy_test_events_contract(
    anvil_url: &str,
) -> Result<(
    Arc<DynProvider>,
    TestEvents::TestEventsInstance<Arc<DynProvider>>,
    String,
)> {
    #[derive(serde::Deserialize)]
    struct SolCompiledOutput {
        bytecode: SolBytecodeOutput,
    }
    #[derive(serde::Deserialize)]
    struct SolBytecodeOutput {
        object: String,
    }

    let bytecode = serde_json::from_slice::<SolCompiledOutput>(TEST_EVENTS_JSON)?
        .bytecode
        .object;
    let bytecode = hex::decode(&bytecode[2..])?;

    let pk: alloy_signer_local::PrivateKeySigner =
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80".parse()?;
    let provider = Arc::new(
        ProviderBuilder::new()
            .with_cached_nonce_management()
            .wallet(pk)
            .connect_http(anvil_url.parse()?)
            .erased(),
    );
    let address = alloy_contract::CallBuilder::new_raw_deploy(&provider, bytecode.into())
        .deploy()
        .await?;
    let contract = TestEvents::TestEventsInstance::new(address, provider.clone());
    let abi_string = serde_json::to_string(&TestEvents::abi::contract())?;
    Ok((provider, contract, abi_string))
}

pub fn create_cross_chain_work_order(
    chain1_url: &str,
    chain2_url: &str,
    contract_abi: &str,
    contract_address: &Address,
) -> Result<DsseEnvelope> {
    let userdata = serde_json::json!({
        "chain1": { "rpcUrl": chain1_url, "contractAddress": contract_address.to_string() },
        "chain2": { "rpcUrl": chain2_url, "contractAddress": contract_address.to_string() },
        "contractAbi": contract_abi,
        "ethereumPrivateKey": "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
    });

    let event = WorkerEvent {
        handler: "valueChanged".to_string(),
        kind: WorkerEventKind::Web3Event(Web3Event {
            chain: 31337,
            address: vec![*contract_address],
            topics: vec![vec![TestEvents::ValueChanged::SIGNATURE_HASH]],
            gateways: vec![chain1_url.replace("http", "ws")],
        }),
    };

    create_work_order("cross_chain_worker.js", Some(userdata), Some(vec![event]))
}

pub fn create_event_counter_work_order(
    chain_url: &str,
    contract_abi: &str,
    contract_address: &Address,
) -> Result<DsseEnvelope> {
    let userdata = serde_json::json!({
        "chain1": { "rpcUrl": chain_url, "contractAddress": contract_address.to_string() },
        "contractAbi": contract_abi,
        "ethereumPrivateKey": "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
    });

    let event = WorkerEvent {
        handler: "valueChanged".to_string(),
        kind: WorkerEventKind::Web3Event(Web3Event {
            chain: 31337,
            address: vec![*contract_address],
            topics: vec![vec![TestEvents::ValueChanged::SIGNATURE_HASH]],
            gateways: vec![chain_url.replace("http", "ws")],
        }),
    };

    create_work_order("event_counter_worker.js", Some(userdata), Some(vec![event]))
}
