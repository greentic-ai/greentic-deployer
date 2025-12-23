use std::io::Write;
use std::path::PathBuf;

use greentic_deployer::bootstrap::cli::DenyPromptAdapter;
use greentic_deployer::bootstrap::flow_runner::run_bootstrap_flow;
use greentic_deployer::platform::{load_bootstrap_flow, load_platform_pack};
use greentic_types::cbor::encode_pack_manifest;
use greentic_types::component::{ComponentCapabilities, ComponentManifest, ComponentProfiles};
use greentic_types::flow::{Flow, FlowKind, FlowMetadata};
use greentic_types::pack_manifest::{BootstrapSpec, PackFlowEntry, PackKind, PackManifest};
use greentic_types::{ComponentId, FlowId, PackId};
use semver::Version;
use serde_json::json;
use tar::Builder;
use tempfile::tempdir;

fn write_stub_gtpack(path: &PathBuf) {
    let manifest = PackManifest {
        schema_version: "pack-v1".to_string(),
        pack_id: PackId::try_from("dev.greentic.platform").unwrap(),
        version: Version::new(0, 1, 0),
        kind: PackKind::Application,
        publisher: "greentic".to_string(),
        components: vec![ComponentManifest {
            id: ComponentId::try_from("dev.greentic.platform.installer").unwrap(),
            version: Version::new(0, 1, 0),
            supports: vec![FlowKind::ComponentConfig],
            world: "greentic:test/world".to_string(),
            profiles: ComponentProfiles::default(),
            capabilities: ComponentCapabilities::default(),
            configurators: None,
            operations: Vec::new(),
            config_schema: None,
            resources: Default::default(),
            dev_flows: Default::default(),
        }],
        flows: vec![pack_flow("platform_install"), pack_flow("platform_upgrade")],
        dependencies: Vec::new(),
        capabilities: Vec::new(),
        secret_requirements: Vec::new(),
        signatures: Default::default(),
        bootstrap: Some(BootstrapSpec {
            install_flow: Some("platform_install".into()),
            upgrade_flow: Some("platform_upgrade".into()),
            installer_component: Some("installer".into()),
        }),
    };

    let encoded_manifest = encode_pack_manifest(&manifest).expect("encode manifest");
    let mut builder = Builder::new(Vec::new());

    // manifest.cbor
    let mut header = tar::Header::new_gnu();
    header.set_size(encoded_manifest.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(&mut header, "manifest.cbor", encoded_manifest.as_slice())
        .expect("append manifest");

    // stub installer.wasm
    let stub = b"stub-wasm";
    let mut wasm_header = tar::Header::new_gnu();
    wasm_header.set_size(stub.len() as u64);
    wasm_header.set_mode(0o644);
    wasm_header.set_cksum();
    builder
        .append_data(
            &mut wasm_header,
            "components/installer.wasm",
            stub.as_slice(),
        )
        .expect("append installer");

    // flows
    let flow_json = json!({
        "steps": [
            {
                "kind": "installer_call",
                "result": {
                    "output_version": "v1",
                    "config_patch": { "telemetry": { "endpoint": "https://otel" } },
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
        let path = format!("flows/{}.ygtc", name);
        builder
            .append_data(&mut flow_header, path, flow_bytes.as_slice())
            .expect("append flow");
    }

    let bytes = builder.into_inner().expect("tar bytes");
    let mut file = std::fs::File::create(path).expect("create gtpack");
    file.write_all(&bytes).expect("write gtpack");
}

fn pack_flow(id: &str) -> PackFlowEntry {
    PackFlowEntry {
        id: FlowId::try_from(id).unwrap(),
        kind: FlowKind::ComponentConfig,
        flow: Flow {
            schema_version: "flowir-v1".to_string(),
            id: FlowId::try_from(id).unwrap(),
            kind: FlowKind::ComponentConfig,
            entrypoints: Default::default(),
            nodes: Default::default(),
            metadata: FlowMetadata::default(),
        },
        tags: Vec::new(),
        entrypoints: Vec::new(),
    }
}

#[test]
fn platform_install_invokes_stub_flow() {
    let dir = tempdir().expect("temp dir");
    let path = dir.path().join("platform.gtpack");
    write_stub_gtpack(&path);

    let info = load_platform_pack(&path).expect("load pack");
    let flow_bytes = load_bootstrap_flow(&path, &info.manifest, true).expect("load bootstrap flow");
    let mut adapter = DenyPromptAdapter;
    let result = run_bootstrap_flow(&flow_bytes, &mut adapter).expect("run bootstrap flow");
    assert!(result.output.ready);
    assert_eq!(
        result.output.config_patch["telemetry"]["endpoint"],
        json!("https://otel")
    );
}
