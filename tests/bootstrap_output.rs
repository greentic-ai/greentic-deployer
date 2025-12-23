use greentic_deployer::bootstrap::cli::DenyPromptAdapter;
use greentic_deployer::bootstrap::flow_runner::run_bootstrap_flow;
use greentic_deployer::bootstrap::output::{BootstrapOutput, SecretWrite};
use serde_json::json;

#[test]
fn output_round_trip_and_redaction() {
    let output = BootstrapOutput::new(
        json!({"telemetry": {"endpoint": "https://otel"}}),
        vec![SecretWrite {
            key: "api_key".into(),
            value: Some("supersecret".into()),
            scope: Some("dev".into()),
            metadata: None,
        }],
        vec!["warning one".into()],
        true,
    );

    let redacted = output.redacted();
    assert_eq!(redacted.secrets_writes[0].value, None);

    let encoded = serde_json::to_string(&output).expect("serialize");
    let decoded: BootstrapOutput = serde_json::from_str(&encoded).expect("deserialize");
    assert_eq!(decoded.output_version, "v1");
    assert!(decoded.ready);
    assert_eq!(decoded.secrets_writes[0].key, "api_key");
    assert_eq!(
        decoded.secrets_writes[0].value.as_deref(),
        Some("supersecret")
    );
}

#[test]
fn flow_runner_parses_bootstrap_output() {
    let flow = json!({
        "steps": [
            {
                "kind": "installer_call",
                "result": {
                    "output_version": "v1",
                    "config_patch": {"a": 1},
                    "secrets_writes": [],
                    "warnings": [],
                    "ready": true
                }
            }
        ]
    });
    let bytes = serde_json::to_vec(&flow).unwrap();
    let mut adapter = DenyPromptAdapter;
    let result = run_bootstrap_flow(&bytes, &mut adapter).expect("run flow");
    assert!(result.output.ready);
    assert_eq!(result.output.config_patch["a"], json!(1));
}
