use std::fs;

use greentic_deployer::bootstrap::cli::DenyPromptAdapter;
use greentic_deployer::bootstrap::cli::JsonPromptAdapter;
use greentic_deployer::bootstrap::flow_runner::run_bootstrap_flow;
use serde_json::json;

#[test]
fn wizard_flow_executes_with_json_answers() {
    let bytes =
        fs::read("fixtures/platform-adapters/multi_step_wizard.ygtc").expect("read wizard flow");
    let answers = json!({
        "region": "eu-west-1",
        "cluster_name": "greentic-edge",
        "admin_email": "alerts@example.com"
    });
    let mut adapter = JsonPromptAdapter::new(answers).expect("answers parsed");
    let result = run_bootstrap_flow(&bytes, &mut adapter).expect("flow executes");
    assert!(result.output.ready);
    assert_eq!(result.output.secrets_writes[0].key, "platform/admin/token");
    // Placeholder values are preserved for installer templating.
    assert_eq!(
        result.output.config_patch["platform"]["control_plane"]["region"],
        json!("{{region}}")
    );
}

#[test]
fn http_fixture_loads_and_returns_endpoints() {
    let bytes =
        fs::read("fixtures/platform-adapters/http_endpoints.ygtc").expect("read http fixture");
    let mut adapter = DenyPromptAdapter;
    let result = run_bootstrap_flow(&bytes, &mut adapter).expect("flow executes");
    assert!(result.output.ready);
    let interaction = &result.output.config_patch["interaction"];
    assert_eq!(interaction["transport"], json!("http"));
    assert_eq!(interaction["schema_endpoint"], json!("GET /schema"));
    assert_eq!(interaction["answers_endpoint"], json!("POST /answers"));
}

#[test]
fn mqtt_fixture_loads_and_returns_topics() {
    let bytes =
        fs::read("fixtures/platform-adapters/mqtt_schema_publish.ygtc").expect("read mqtt fixture");
    let mut adapter = DenyPromptAdapter;
    let result = run_bootstrap_flow(&bytes, &mut adapter).expect("flow executes");
    assert!(result.output.ready);
    let topics = &result.output.config_patch["interaction"]["topics"];
    assert_eq!(topics["schema"], json!("greentic/bootstrap/edge-01/schema"));
    assert_eq!(
        topics["answers"],
        json!("greentic/bootstrap/edge-01/answers")
    );
}
