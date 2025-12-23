use std::path::PathBuf;

use greentic_deployer::bootstrap::state::{
    BootstrapState, ensure_upgrade_allowed, load_state, save_state,
};
use semver::Version;
use tempfile::tempdir;

#[test]
fn missing_state_returns_none() {
    let dir = tempdir().expect("temp dir");
    let path = dir.path().join("state.json");
    assert!(load_state(&path).expect("load missing").is_none());
}

#[test]
fn roundtrip_state() {
    let dir = tempdir().expect("temp dir");
    let path: PathBuf = dir.path().join("state.json");
    let state = BootstrapState {
        version: Some("0.1.0".into()),
        digest: Some("sha256:abc".into()),
        installed_at: Some(123),
        environment_kind: Some("local".into()),
        last_upgrade_at: Some(456),
        rollback_ref: Some("ref1".into()),
    };
    save_state(&path, &state).expect("save state");
    let loaded = load_state(&path)
        .expect("load state")
        .expect("state present");
    assert_eq!(loaded, state);
}

#[test]
fn upgrade_preflight_blocks_missing_state_and_downgrade() {
    let target = Version::parse("1.2.0").unwrap();
    let err = ensure_upgrade_allowed(None, &target).expect_err("missing state should fail");
    assert!(format!("{err}").contains("not installed"));

    let state = BootstrapState::installed_now(Some("1.2.0".into()), Some("abc".into()));
    let err = ensure_upgrade_allowed(Some(state), &Version::parse("1.1.0").unwrap())
        .expect_err("downgrade should fail");
    assert!(format!("{err}").contains("newer pack version"));
}

#[test]
fn upgrade_state_sets_last_upgrade_timestamp() {
    let state = BootstrapState::installed_now(Some("1.0.0".into()), Some("abc".into()));
    let upgraded = BootstrapState::upgraded_from(
        &state,
        Some("1.1.0".into()),
        Some("def".into()),
        Some("rollback".to_string()),
    );
    assert_eq!(upgraded.version.as_deref(), Some("1.1.0"));
    assert!(upgraded.last_upgrade_at.is_some());
    assert_eq!(upgraded.digest.as_deref(), Some("def"));
    assert_eq!(upgraded.rollback_ref.as_deref(), Some("rollback"));
}
