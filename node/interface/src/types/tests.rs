use alloy_primitives::{Address, U256};
use chrono::DateTime;
use serde_json::Value;
use url::Url;

use super::{
    secrets::{ChainIdentifier, SecretId},
    worker::{
        WorkOrderPayload, WorkerBundlePointer, WorkerManifest,
        events::{Mode, RateMode, Schedule, Web3Event, WorkerEvent, WorkerEventKind},
    },
};
use crate::proto::interface;

#[test]
fn test_work_order_payload_deserialization_with_complex_userdata() {
    let json_str = r#"{
            "id": "test-wo-123",
            "worker": {
                "bundle": {
                    "source": "data:application/json;base64,e30="
                },
                "identities": [],
                "userdata": {
                    "config_string": "some-value",
                    "config_number": 123,
                    "config_bool": true,
                    "config_object": {
                        "nested_key": "nested_value",
                        "nested_array": [1, "two", false]
                    }
                }
            },
            "events": [
                { "handler": "onLaunch", "kind": "launch" },
                { "handler": "onEvent", "kind": "web3_event", "chain": 1, "address": [], "topics": [] },
                { "handler": "onScheduled", "kind": "scheduled", "period_ms": 60000 }
            ]
        }"#;

    let result: Result<WorkOrderPayload, _> = serde_json::from_str(json_str);
    assert!(result.is_ok(), "Failed to deserialize: {:?}", result.err());

    let payload = result.unwrap();
    assert_eq!(payload.id, "test-wo-123");
    assert_eq!(payload.events.len(), 3);

    let userdata = &payload.worker.userdata;
    assert_eq!(
        userdata.get("config_string").unwrap(),
        &Value::String("some-value".to_string())
    );
    assert_eq!(
        userdata.get("config_number").unwrap(),
        &Value::Number(123.into())
    );
    assert!(userdata.get("config_object").unwrap().is_object());

    // Verify the scheduled event was parsed correctly
    if let WorkerEventKind::Scheduled(Schedule::Rate(rate_mode)) = &payload.events[2].kind {
        assert_eq!(rate_mode.period_ms, 60000);
        assert_eq!(rate_mode.mode, Mode::Rate);
    } else {
        panic!("Expected scheduled event kind");
    }
}

#[test]
fn test_work_order_with_scheduled_events() {
    // Test a full work order with scheduled events to ensure it serializes/deserializes correctly
    let work_order = WorkOrderPayload {
        id: "test-scheduled".to_string(),
        worker: WorkerManifest {
            bundle: WorkerBundlePointer {
                source: url::Url::parse(
                    "data:application/javascript;base64,Y29uc29sZS5sb2coImhlbGxvIik=",
                )
                .unwrap(),
                hash: None,
            },
            identities: vec![],
            userdata: std::collections::HashMap::new(),
        },
        events: vec![
            WorkerEvent {
                handler: "launch".to_string(),
                kind: WorkerEventKind::Launch,
            },
            WorkerEvent {
                handler: "tick".to_string(),
                kind: WorkerEventKind::Scheduled(Schedule::Rate(RateMode::new(5000))),
            },
        ],
    };

    // Test serialization
    let json = serde_json::to_string_pretty(&work_order).expect("Serialization should work");
    println!("Work order JSON: {}", json);

    // Test deserialization
    let deserialized: WorkOrderPayload =
        serde_json::from_str(&json).expect("Deserialization should work");
    assert_eq!(deserialized.id, work_order.id);
    assert_eq!(deserialized.events.len(), 2);

    // Verify the scheduled event was preserved
    if let WorkerEventKind::Scheduled(Schedule::Rate(rate_mode)) = &deserialized.events[1].kind {
        assert_eq!(rate_mode.period_ms, 5000);
    } else {
        panic!("Expected scheduled event");
    }
}

#[test]
fn test_schedule_serialization_deserialization() {
    use super::worker::events::{CatchUp, Policy};
    // Test minimal schedule
    let schedule = Schedule::Rate(RateMode::new(1000));
    let json = serde_json::to_string(&schedule).unwrap();
    let deserialized: Schedule = serde_json::from_str(&json).unwrap();
    assert_eq!(schedule, deserialized);

    // Test full schedule with all options
    let start_time = DateTime::from_timestamp(1640995200, 0).unwrap(); // 2022-01-01 00:00:00 UTC
    let end_time = DateTime::from_timestamp(1672531200, 0).unwrap(); // 2023-01-01 00:00:00 UTC

    let policy = Policy {
        catch_up: CatchUp::Coalesce,
        max_lateness_ms: Some(5000),
        jitter_budget_ms: Some(100),
    };

    let rate_mode = RateMode {
        mode: Mode::Rate,
        period_ms: 30000,
        phase_ms: 1000,
        start_at: Some(start_time),
        end_at: Some(end_time),
        max_occurrences: Some(100),
        policy: Some(policy),
    };

    let schedule = Schedule::Rate(rate_mode);
    let json = serde_json::to_string(&schedule).unwrap();
    let deserialized: Schedule = serde_json::from_str(&json).unwrap();
    assert_eq!(schedule, deserialized);

    // Test that minimal JSON works (only period_ms specified)
    let minimal_json = r#"{"period_ms": 5000}"#;
    let schedule: Schedule = serde_json::from_str(minimal_json).unwrap();
    let Schedule::Rate(rate_mode) = schedule;
    assert_eq!(rate_mode.period_ms, 5000);
    assert_eq!(rate_mode.mode, Mode::Rate);
    assert_eq!(rate_mode.phase_ms, 0);
    assert!(rate_mode.start_at.is_none());
    assert!(rate_mode.end_at.is_none());
    assert!(rate_mode.max_occurrences.is_none());
    assert!(rate_mode.policy.is_none());
}

#[test]
fn test_chain_identifier_serialization() {
    // Test JSON serialization backwards compatibility
    let chain_id = ChainIdentifier::ChainId(1);
    let json = serde_json::to_string(&chain_id).unwrap();
    assert_eq!(json, "1");

    let gateway = ChainIdentifier::GatewayUrl("wss://custom.com".parse().unwrap());
    let json = serde_json::to_string(&gateway).unwrap();
    assert_eq!(json, "\"wss://custom.com/\"");
}

#[test]
fn test_chain_identifier_deserialization_edge_cases() {
    // Test invalid URLs
    let result: Result<ChainIdentifier, _> = serde_json::from_str("\"not-a-url\"");
    assert!(result.is_err());

    // Test various URL schemes
    let schemes = ["ws://", "wss://", "http://", "https://"];
    for scheme in schemes {
        let url = format!("\"{}example.com\"", scheme);
        let result: Result<ChainIdentifier, _> = serde_json::from_str(&url);
        assert!(result.is_ok());
    }
}

#[test]
fn test_chain_identifier_display() {
    let chain_id = ChainIdentifier::ChainId(42);
    assert_eq!(chain_id.to_string(), "42");

    let gateway = ChainIdentifier::GatewayUrl("wss://test.com".parse().unwrap());
    assert_eq!(gateway.to_string(), "wss://test.com/");
}

#[test]
fn test_chain_identifier_helper_methods() {
    let chain_id = ChainIdentifier::ChainId(123);
    assert_eq!(chain_id.chain_id(), Some(123));
    assert_eq!(chain_id.gateway_url(), None);

    let url: Url = "wss://example.com".parse().unwrap();
    let gateway = ChainIdentifier::GatewayUrl(url.clone());
    assert_eq!(gateway.chain_id(), None);
    assert_eq!(gateway.gateway_url(), Some(&url));
}

#[test]
fn test_chain_identifier_protobuf_conversion() {
    // Test ChainId conversion
    let chain_id = ChainIdentifier::ChainId(42);
    let proto: interface::ChainIdentifier = chain_id.clone().into();
    let back: ChainIdentifier = proto.try_into().unwrap();
    assert_eq!(chain_id, back);

    // Test GatewayUrl conversion
    let gateway = ChainIdentifier::GatewayUrl("wss://test.com".parse().unwrap());
    let proto: interface::ChainIdentifier = gateway.clone().into();
    let back: ChainIdentifier = proto.try_into().unwrap();
    assert_eq!(gateway, back);
}

#[test]
fn test_protobuf_conversion_errors() {
    // Test missing identifier field
    let proto = interface::ChainIdentifier { identifier: None };
    let result: Result<ChainIdentifier, _> = proto.try_into();
    assert!(result.is_err());

    // Test invalid URL in protobuf
    let proto = interface::ChainIdentifier {
        identifier: Some(interface::chain_identifier::Identifier::GatewayUrl(
            "not-a-valid-url".to_string(),
        )),
    };
    let result: Result<ChainIdentifier, _> = proto.try_into();
    assert!(result.is_err());
}

#[test]
fn test_secret_id_with_custom_gateway() {
    let gateway_url = "wss://custom.chain.com".parse().unwrap();
    let secret_id = SecretId {
        chain: ChainIdentifier::GatewayUrl(gateway_url),
        identity_address: Address::from([1u8; 20]),
        identity_id: U256::from(456),
    };

    // Test serialization roundtrip
    let json = serde_json::to_string(&secret_id).unwrap();
    let deserialized: SecretId = serde_json::from_str(&json).unwrap();
    assert_eq!(secret_id, deserialized);
}

#[test]
fn test_web3_event_with_custom_gateway() {
    let gateway_url = "wss://custom.chain.com".parse().unwrap();
    let event = Web3Event {
        chain: ChainIdentifier::GatewayUrl(gateway_url),
        address: vec![],
        topics: vec![],
        gateways: vec![],
    };

    // Test that custom gateway works in event context
    let json = serde_json::to_string(&event).unwrap();
    let deserialized: Web3Event = serde_json::from_str(&json).unwrap();
    assert_eq!(event, deserialized);
}
