use std::io::Write;
use std::path::PathBuf;

use greentic_deployer::bootstrap::config_patch::apply_config_patch;
use greentic_deployer::bootstrap::flow_runner::run_bootstrap_flow;
use greentic_deployer::bootstrap::state::{BootstrapState, load_state, save_state};
use greentic_deployer::platform::{load_bootstrap_flow, load_platform_pack};
use serde_json::json;
use tar::Builder;
use tempfile::tempdir;

fn write_stub_pack(path: &PathBuf) {
    let manifest = greentic_types::pack_manifest::PackManifest {
        schema_version: "pack-v1".to_string(),
        pack_id: greentic_types::PackId::try_from("dev.greentic.platform").unwrap(),
        version: semver::Version::new(0, 1, 0),
        kind: greentic_types::pack_manifest::PackKind::Application,
        publisher: "greentic".to_string(),
        components: Vec::new(),
        flows: vec![pack_flow("platform_install"), pack_flow("platform_upgrade")],
        dependencies: Vec::new(),
        capabilities: Vec::new(),
        secret_requirements: Vec::new(),
        signatures: Default::default(),
        bootstrap: Some(greentic_types::pack_manifest::BootstrapSpec {
            install_flow: Some("platform_install".into()),
            upgrade_flow: None,
            installer_component: None,
        }),
        extensions: None,
    };
    let manifest_bytes =
        greentic_types::cbor::encode_pack_manifest(&manifest).expect("encode manifest");
    let mut builder = Builder::new(Vec::new());

    // manifest.cbor
    let mut header = tar::Header::new_gnu();
    header.set_size(manifest_bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(&mut header, "manifest.cbor", manifest_bytes.as_slice())
        .expect("append manifest");

    // install flow
    let flow_json = json!({
        "steps": [
            {
                "kind": "installer_call",
                "result": {
                    "output_version": "v1",
                    "config_patch": { "platform": { "enabled": true } },
                    "secrets_writes": [],
                    "warnings": [],
                    "ready": true
                }
            }
        ]
    });
    let flow_bytes = serde_json::to_vec(&flow_json).unwrap();
    for name in &["platform_install", "platform_upgrade"] {
        let mut flow_header = tar::Header::new_gnu();
        flow_header.set_size(flow_bytes.len() as u64);
        flow_header.set_mode(0o644);
        flow_header.set_cksum();
        let path = format!("flows/{name}.ygtc");
        builder
            .append_data(&mut flow_header, path, flow_bytes.as_slice())
            .expect("append flow");
    }

    let bytes = builder.into_inner().expect("tar bytes");
    let mut file = std::fs::File::create(path).expect("create gtpack");
    file.write_all(&bytes).expect("write gtpack");
}

fn pack_flow(id: &str) -> greentic_types::pack_manifest::PackFlowEntry {
    greentic_types::pack_manifest::PackFlowEntry {
        id: greentic_types::FlowId::try_from(id).unwrap(),
        kind: greentic_types::flow::FlowKind::ComponentConfig,
        flow: greentic_types::flow::Flow {
            schema_version: "flowir-v1".to_string(),
            id: greentic_types::FlowId::try_from(id).unwrap(),
            kind: greentic_types::flow::FlowKind::ComponentConfig,
            entrypoints: Default::default(),
            nodes: Default::default(),
            metadata: greentic_types::flow::FlowMetadata::default(),
        },
        tags: Vec::new(),
        entrypoints: Vec::new(),
    }
}

#[test]
fn config_patch_written_and_state_saved_on_success() {
    let dir = tempdir().expect("temp dir");
    let pack_path = dir.path().join("platform.gtpack");
    write_stub_pack(&pack_path);

    let config_path = dir.path().join("config_patch.json");
    let state_path = dir.path().join("state.json");

    let info = load_platform_pack(&pack_path).expect("load pack");
    let flow_bytes =
        load_bootstrap_flow(&pack_path, &info.manifest, true).expect("load bootstrap flow");
    let mut adapter = greentic_deployer::bootstrap::cli::JsonPromptAdapter::new(json!({}))
        .expect("empty answers ok");
    let result = run_bootstrap_flow(&flow_bytes, &mut adapter).expect("run flow");

    apply_config_patch(&config_path, &result.output.config_patch).expect("write config patch");
    save_state(
        &state_path,
        &BootstrapState::installed_now(
            Some(info.manifest.version.to_string()),
            info.digest.clone(),
        ),
    )
    .expect("save state");

    let written = std::fs::read_to_string(&config_path).expect("read config patch");
    assert!(written.contains("\"enabled\": true"));

    let state = load_state(&state_path)
        .expect("load state")
        .expect("some state");
    assert_eq!(state.version.as_deref(), Some("0.1.0"));
}

#[test]
fn state_not_saved_when_config_write_fails() {
    let dir = tempdir().expect("temp dir");
    let invalid_path = dir.path().join("config_dir");
    std::fs::create_dir_all(&invalid_path).expect("create dir");

    let err = apply_config_patch(&invalid_path, &json!({"a": 1})).expect_err("should fail");
    assert!(format!("{err}").contains("Is a directory"));

    let state_path = dir.path().join("state.json");
    assert!(load_state(&state_path).expect("load state").is_none());
}
