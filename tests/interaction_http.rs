use std::thread;
use std::time::Duration;

use greentic_deployer::bootstrap::flow_runner::run_bootstrap_flow;
use greentic_deployer::bootstrap::http_adapter::HttpPromptAdapter;
use serde_json::json;

#[test]
fn http_adapter_serves_schema_and_accepts_answers() {
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
    let bytes = serde_json::to_vec(&flow).unwrap();

    let mut adapter = match HttpPromptAdapter::bind("127.0.0.1:0", Duration::from_secs(5)) {
        Ok(adapter) => adapter,
        Err(err) => {
            // Some environments disallow binding listeners; skip in that case.
            eprintln!("http adapter bind skipped: {err}");
            return;
        }
    };
    let addr = adapter.bound_addr();

    thread::spawn(move || {
        // give server a moment to start
        std::thread::sleep(Duration::from_millis(50));
        let client = reqwest::blocking::Client::new();
        let schema_url = format!("http://{}/schema", addr);
        let answers_url = format!("http://{}/answers", addr);

        let schema = client
            .get(&schema_url)
            .send()
            .expect("schema request")
            .json::<serde_json::Value>()
            .expect("schema json");
        assert_eq!(schema["questions"][0]["id"], json!("region"));

        let resp = client
            .post(&answers_url)
            .body(r#"{"region":"eu-west-1"}"#)
            .send()
            .expect("post answers");
        assert!(resp.status().is_success());
    });

    let result = run_bootstrap_flow(&bytes, &mut adapter).expect("run flow");
    assert_eq!(result.output.config_patch["region"], json!("{{region}}"));
    assert!(result.output.ready);
}
