use greentic_deployer::platform::flow::{BootstrapResolution, resolve_bootstrap};
use greentic_types::component::{ComponentCapabilities, ComponentManifest, ComponentProfiles};
use greentic_types::flow::{Flow, FlowKind, FlowMetadata};
use greentic_types::pack_manifest::{PackFlowEntry, PackKind, PackManifest};
use greentic_types::{FlowId, PackId};
use semver::Version;

fn flow(id: &str) -> PackFlowEntry {
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

fn manifest_with_bootstrap() -> PackManifest {
    PackManifest {
        schema_version: "pack-v1".to_string(),
        pack_id: PackId::try_from("dev.greentic.platform").unwrap(),
        version: Version::new(0, 1, 0),
        kind: PackKind::Application,
        publisher: "greentic".to_string(),
        components: vec![ComponentManifest {
            id: "dev.greentic.platform.installer".try_into().unwrap(),
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
        flows: vec![flow("custom_install"), flow("custom_upgrade")],
        dependencies: Vec::new(),
        capabilities: Vec::new(),
        secret_requirements: Vec::new(),
        signatures: Default::default(),
        bootstrap: Some(greentic_types::pack_manifest::BootstrapSpec {
            install_flow: Some("custom_install".into()),
            upgrade_flow: Some("custom_upgrade".into()),
            installer_component: Some("custom_installer".into()),
        }),
    }
}

fn manifest_without_bootstrap() -> PackManifest {
    let mut manifest = manifest_with_bootstrap();
    manifest.bootstrap = None;
    manifest.flows = vec![flow("platform_install"), flow("platform_upgrade")];
    manifest
}

#[test]
fn resolves_explicit_bootstrap_fields() {
    let manifest = manifest_with_bootstrap();
    let resolved = resolve_bootstrap(&manifest).expect("resolve bootstrap");
    assert_eq!(
        resolved,
        BootstrapResolution {
            install_flow: "custom_install".into(),
            upgrade_flow: "custom_upgrade".into(),
            installer_component: "custom_installer".into(),
        }
    );
}

#[test]
fn falls_back_to_defaults_when_missing_bootstrap_block() {
    let manifest = manifest_without_bootstrap();
    let resolved = resolve_bootstrap(&manifest).expect("resolve bootstrap");
    assert_eq!(resolved.install_flow, "platform_install");
    assert_eq!(resolved.upgrade_flow, "platform_upgrade");
    assert_eq!(resolved.installer_component, "installer");
}

#[test]
fn errors_when_flow_missing() {
    let mut manifest = manifest_without_bootstrap();
    manifest.flows.clear();
    let err = resolve_bootstrap(&manifest).unwrap_err();
    assert!(
        err.contains("bootstrap flow"),
        "expected missing flow error, got {err}"
    );
}
