use std::fs;

use greentic_deployer::bootstrap::output::SecretWrite;
use greentic_deployer::bootstrap::secrets::{
    execute_writes, parse_backend, set_k8s_secret_dir_override,
};
use serde_json::json;
use tempfile::tempdir;

#[test]
fn writes_secrets_to_file_backend() {
    let dir = tempdir().expect("temp dir");
    let path = dir.path().join("secrets.db");
    let backend =
        parse_backend(&format!("file:{}", path.display())).expect("parse file backend succeeds");

    let writes = vec![SecretWrite {
        key: "api_key".into(),
        value: Some("supersecret".into()),
        scope: Some("dev".into()),
        metadata: Some(json!({"source": "installer"})),
    }];

    execute_writes(&backend, &writes).expect("write secrets");

    let content = fs::read_to_string(&path).expect("secrets file exists");
    let parsed: serde_json::Value = serde_json::from_str(&content).expect("json");
    let entry = parsed
        .get("dev/api_key")
        .expect("dev/api_key stored in file backend");
    assert_eq!(entry["value"], json!("supersecret"));
    assert_eq!(entry["scope"], json!("dev"));
    assert_eq!(entry["metadata"]["source"], json!("installer"));
}

#[test]
fn errors_on_unknown_backend() {
    let err = parse_backend("k8s:foo").expect_err("unknown backend should error");
    assert!(format!("{err}").contains("k8s backend expects"));
}

#[test]
fn writes_secrets_to_k8s_backend_stub() {
    let dir = tempdir().expect("temp dir");
    set_k8s_secret_dir_override(Some(dir.path().to_path_buf()));
    let backend = parse_backend("k8s:dev/greentic-bootstrap").expect("parse k8s backend succeeds");
    let writes = vec![SecretWrite {
        key: "api_key".into(),
        value: Some("supersecret".into()),
        scope: Some("dev".into()),
        metadata: None,
    }];

    execute_writes(&backend, &writes).expect("write secrets");

    let secret_path = dir.path().join("dev").join("greentic-bootstrap.yaml");
    let content = std::fs::read_to_string(secret_path).expect("secret file exists");
    let parsed: serde_json::Value = serde_yaml_bw::from_str(&content).expect("yaml parses to json");
    use base64::Engine;
    use base64::engine::general_purpose;
    let decoded = parsed["data"]["dev/api_key"]
        .as_str()
        .and_then(|s| general_purpose::STANDARD.decode(s).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap();
    assert_eq!(decoded, "supersecret");
}
