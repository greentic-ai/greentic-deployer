use greentic_deployer::bootstrap::cli::JsonPromptAdapter;
use greentic_deployer::bootstrap::flow_runner::run_bootstrap_flow;
use serde_json::json;

#[test]
fn json_adapter_supplies_answers() {
    let flow = json!({
        "steps": [
            {
                "kind": "prompt",
                "questions": [
                    { "id": "region", "prompt": "Region?", "default": "us-east-1" }
                ]
            },
            {
                "kind": "installer_call",
                "result": {
                    "output_version": "v1",
                    "config_patch": {"region": "{{region}}"},
                    "secrets_writes": [],
                    "warnings": [],
                    "ready": true
                }
            }
        ]
    });

    let answers = json!({"region": "eu-west-1"});
    let mut adapter = JsonPromptAdapter::new(answers).expect("valid answers");
    let bytes = serde_json::to_vec(&flow).unwrap();

    let result = run_bootstrap_flow(&bytes, &mut adapter).expect("flow runs");
    assert!(result.output.ready);
    assert_eq!(result.output.config_patch["region"], json!("{{region}}"));
}
