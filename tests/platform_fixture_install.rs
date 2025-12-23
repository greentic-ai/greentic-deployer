use std::fs;
use std::path::PathBuf;

use greentic_deployer::bootstrap::cli::JsonPromptAdapter;
use greentic_deployer::bootstrap::flow_runner::run_bootstrap_flow;
use greentic_deployer::platform::{load_bootstrap_flow, load_platform_pack};
use greentic_types::cbor::encode_pack_manifest;
use greentic_types::pack_manifest::PackManifest;
use serde_json::json;
use tar::Builder;
use tempfile::tempdir;

fn build_fixture_gtpack() -> (tempfile::TempDir, PathBuf) {
    let dir = tempdir().expect("temp dir");
    let pack_path = dir.path().join("platform.gtpack");

    let manifest_yaml =
        fs::read_to_string("fixtures/platform-pack/pack.yaml").expect("read fixture pack.yaml");
    let manifest: PackManifest =
        serde_yaml_bw::from_str(&manifest_yaml).expect("parse pack.yaml into manifest");
    let manifest_bytes = encode_pack_manifest(&manifest).expect("encode manifest");

    let install_flow =
        fs::read("fixtures/platform-pack/flows/platform_install.ygtc").expect("read install flow");
    let upgrade_flow =
        fs::read("fixtures/platform-pack/flows/platform_upgrade.ygtc").expect("read upgrade flow");
    let installer_wasm =
        fs::read("fixtures/platform-pack/installer.wasm").expect("read installer wasm");

    let mut builder = Builder::new(Vec::new());

    // manifest.cbor
    let mut header = tar::Header::new_gnu();
    header.set_size(manifest_bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(&mut header, "manifest.cbor", manifest_bytes.as_slice())
        .expect("append manifest");

    // flows
    for (name, bytes) in [
        ("platform_install", install_flow),
        ("platform_upgrade", upgrade_flow),
    ] {
        let mut h = tar::Header::new_gnu();
        h.set_size(bytes.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        let path = format!("flows/{name}.ygtc");
        builder
            .append_data(&mut h, path, bytes.as_slice())
            .expect("append flow");
    }

    // installer.wasm
    let mut wasm_header = tar::Header::new_gnu();
    wasm_header.set_size(installer_wasm.len() as u64);
    wasm_header.set_mode(0o644);
    wasm_header.set_cksum();
    builder
        .append_data(
            &mut wasm_header,
            "components/installer.wasm",
            installer_wasm.as_slice(),
        )
        .expect("append installer");

    let bytes = builder.into_inner().expect("tar bytes");
    fs::write(&pack_path, bytes).expect("write gtpack");

    (dir, pack_path)
}

#[test]
fn platform_install_uses_fixture_pack_with_json_answers() {
    let (_dir, pack_path) = build_fixture_gtpack();

    let info = load_platform_pack(&pack_path).expect("load pack");
    let flow_bytes = load_bootstrap_flow(&pack_path, &info.manifest, true).expect("load flow");

    let answers = json!({"region": "eu-west-1"});
    let mut adapter = JsonPromptAdapter::new(answers).expect("valid answers");
    let result = run_bootstrap_flow(&flow_bytes, &mut adapter).expect("run bootstrap flow");

    assert!(result.output.ready);
    assert_eq!(
        result.output.config_patch["platform"]["control_plane"]["cluster"],
        json!("greentic-platform")
    );
    assert_eq!(result.output.secrets_writes.len(), 1);
}
