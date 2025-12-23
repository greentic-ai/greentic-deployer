use greentic_deployer::bootstrap::state::{BootstrapState, load_state_backend, save_state_backend};
use greentic_deployer::config::BootstrapStateBackend;
use tempfile::tempdir;

#[test]
fn file_backend_reads_and_writes() {
    let dir = tempdir().expect("temp dir");
    let path = dir.path().join("state.json");
    let state = BootstrapState::installed_now(Some("1.2.3".into()), Some("sha256:abc".into()));

    save_state_backend(BootstrapStateBackend::File, &path, "ns", "name", &state)
        .expect("save state");

    let loaded =
        load_state_backend(BootstrapStateBackend::File, &path, "ns", "name").expect("load state");
    assert_eq!(loaded.as_ref().unwrap().version.as_deref(), Some("1.2.3"));
}

#[test]
fn k8s_backend_not_available_errors() {
    let dir = tempdir().expect("temp dir");
    let path = dir.path().join("state.json");
    let err = load_state_backend(BootstrapStateBackend::K8s, &path, "ns", "name")
        .expect_err("k8s backend should error");
    assert!(format!("{err}").contains("not available"));
}
