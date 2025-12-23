use greentic_deployer::bootstrap::flow_runner::run_bootstrap_flow;
use greentic_deployer::bootstrap::mqtt_adapter::{MockBroker, MqttPromptAdapter};
use greentic_deployer::bootstrap::network::{NetAllowList, NetworkPolicy};
use serde_json::json;
use std::thread;
use std::time::Duration;

fn flow_bytes() -> Vec<u8> {
    let flow = json!({
        "steps": [
            {
                "kind": "prompt",
                "questions": [
                    { "id": "device_name", "prompt": "Device name?", "default": "edge-1" }
                ]
            },
            {
                "kind": "installer_call",
                "result": {
                    "output_version": "v1",
                    "config_patch": {"device": {"name": "{{device_name}}"}},
                    "secrets_writes": [],
                    "warnings": [],
                    "ready": true
                }
            }
        ]
    });
    serde_json::to_vec(&flow).unwrap()
}

#[test]
fn mqtt_mock_adapter_exchanges_schema_and_answers() {
    let broker = MockBroker::default();
    let mut adapter = MqttPromptAdapter::new_mock(
        broker.clone(),
        "device-123".into(),
        "greentic/install".into(),
    )
    .expect("adapter")
    .with_timeout(Duration::from_secs(2));

    // Subscribe before running to capture schema/status.
    let schema_rx = broker.subscribe("greentic/install/device-123/schema");
    let status_rx = broker.subscribe("greentic/install/device-123/status");

    let bytes = flow_bytes();

    // Client thread: wait for schema then publish answers.
    thread::spawn(move || {
        let schema = schema_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("schema msg");
        let schema_json: serde_json::Value = serde_json::from_slice(&schema).expect("schema json");
        assert_eq!(schema_json["questions"][0]["id"], json!("device_name"));

        let answers = json!({"device_name": "edge-west"});
        let payload = serde_json::to_vec(&answers).unwrap();
        broker.publish("greentic/install/device-123/answers", &payload);
    });

    let result = run_bootstrap_flow(&bytes, &mut adapter).expect("run flow");
    assert_eq!(
        result.output.config_patch["device"]["name"],
        json!("{{device_name}}")
    );
    assert!(result.output.ready);

    // status published
    let status = status_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("status msg");
    let status_json: serde_json::Value = serde_json::from_slice(&status).expect("status json");
    assert_eq!(status_json["status"], json!("answers_received"));
}

#[test]
fn mqtt_adapter_blocks_unallowlisted_broker() {
    let broker = MockBroker::default();
    let policy = NetworkPolicy::new(true, false, NetAllowList::default());
    let err = MqttPromptAdapter::new_mock(
        broker.clone(),
        "device-123".into(),
        "greentic/install".into(),
    )
    .expect("adapter")
    .with_network_policy(policy, "mqtt.example.com".into());
    match err {
        Ok(_) => panic!("expected unallowlisted broker to be blocked"),
        Err(e) => assert!(e.to_string().contains("allowlist")),
    }
}

#[test]
fn mqtt_adapter_allows_allowlisted_broker() {
    let broker = MockBroker::default();
    let allowlist =
        NetAllowList::parse(Some("mqtt.example.com,10.0.0.0/8")).expect("allowlist parse");
    let policy = NetworkPolicy::new(true, false, allowlist);
    let mut adapter = MqttPromptAdapter::new_mock(
        broker.clone(),
        "device-allow".into(),
        "greentic/install".into(),
    )
    .expect("adapter")
    .with_network_policy(policy, "mqtt.example.com".into())
    .expect("allowed broker")
    .with_timeout(Duration::from_secs(2));

    // Pre-subscribe to see schema/status.
    let schema_rx = broker.subscribe("greentic/install/device-allow/schema");
    let answers_rx = broker.subscribe("greentic/install/device-allow/status");

    let bytes = flow_bytes();
    thread::spawn(move || {
        let schema = schema_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("schema msg");
        let schema_json: serde_json::Value = serde_json::from_slice(&schema).expect("schema json");
        assert_eq!(schema_json["questions"][0]["id"], json!("device_name"));

        let answers = json!({"device_name": "edge-east"});
        let payload = serde_json::to_vec(&answers).unwrap();
        broker.publish("greentic/install/device-allow/answers", &payload);
    });

    let result = run_bootstrap_flow(&bytes, &mut adapter).expect("run flow");
    assert!(result.output.ready);

    let status = answers_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("status msg");
    let status_json: serde_json::Value = serde_json::from_slice(&status).expect("status json");
    assert_eq!(status_json["status"], json!("answers_received"));
}
