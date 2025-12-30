use greentic_deployer::provider_onboarding::{self, OnboardRequest};
use greentic_types::cbor::encode_pack_manifest;
use greentic_types::component::{ComponentCapabilities, ComponentManifest, ComponentProfiles};
use greentic_types::pack_manifest::{ExtensionRef, PackKind, PackManifest};
use greentic_types::{
    ComponentId, PackId, ProviderDecl, ProviderExtensionInline, ProviderRuntimeRef,
};
use semver::Version;
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use tempfile::tempdir;

#[test]
fn onboard_dummy_provider_pack() {
    let dir = tempdir().expect("temp dir");
    let pack_dir = dir.path().join("pack");
    fs::create_dir_all(pack_dir.join("components")).unwrap();
    fs::create_dir_all(pack_dir.join("schemas")).unwrap();

    let provider_runtime = "dev.greentic.provider.core";
    let provider_type = "vendor.echo";

    let mut extensions = BTreeMap::new();
    let providers = ProviderExtensionInline {
        providers: vec![ProviderDecl {
            provider_type: provider_type.into(),
            capabilities: vec!["messaging".into()],
            ops: vec!["validate-config".into()],
            config_schema_ref: "schemas/provider-config.json".into(),
            state_schema_ref: None,
            runtime: ProviderRuntimeRef {
                component_ref: provider_runtime.into(),
                export: "validate-config".into(),
                world: "greentic:provider-schema-core/schema-core@1.0.0".into(),
            },
            docs_ref: None,
        }],
    };
    extensions.insert(
        "greentic.ext.provider".into(),
        ExtensionRef {
            kind: "greentic.ext.provider".into(),
            version: "1.0.0".into(),
            digest: None,
            location: None,
            inline: Some(serde_json::to_value(providers).unwrap()),
        },
    );

    let manifest = PackManifest {
        schema_version: "pack-v1".into(),
        pack_id: PackId::try_from("dev.greentic.provider").unwrap(),
        version: Version::new(0, 1, 0),
        kind: PackKind::Provider,
        publisher: "greentic".into(),
        components: vec![ComponentManifest {
            id: ComponentId::try_from(provider_runtime).unwrap(),
            version: Version::new(0, 1, 0),
            supports: Vec::new(),
            world: "greentic:provider-schema-core/schema-core@1.0.0".into(),
            profiles: ComponentProfiles::default(),
            capabilities: ComponentCapabilities::default(),
            configurators: None,
            operations: Vec::new(),
            config_schema: None,
            resources: Default::default(),
            dev_flows: Default::default(),
        }],
        flows: Vec::new(),
        dependencies: Vec::new(),
        capabilities: Vec::new(),
        secret_requirements: Vec::new(),
        signatures: Default::default(),
        bootstrap: None,
        extensions: Some(extensions),
    };

    let manifest_bytes = encode_pack_manifest(&manifest).unwrap();
    fs::write(pack_dir.join("manifest.cbor"), manifest_bytes).unwrap();
    let pack_path = pack_dir.canonicalize().unwrap();

    let schema = json!({
        "type": "object",
        "required": ["token"],
        "properties": {
            "token": {"type": "string", "format": "password"},
            "region": {"type": "string", "enum": ["us", "eu"], "default": "us"}
        }
    });
    fs::write(
        pack_dir.join("schemas/provider-config.json"),
        serde_json::to_vec_pretty(&schema).unwrap(),
    )
    .unwrap();

    let component_wat = r#"
        (module
          (memory (export "memory") 1)
          (data (i32.const 0) "{\"valid\":true}")
          (func (export "validate-config") (result i32 i32)
            i32.const 0
            i32.const 14)
        )
    "#;
    let wasm = wat::parse_str(component_wat).expect("wat parses");
    fs::write(
        pack_dir
            .join("components")
            .join(format!("{provider_runtime}.wasm")),
        wasm,
    )
    .unwrap();

    let config_path = dir.path().join("config.json");
    fs::write(
        &config_path,
        serde_json::to_vec_pretty(&json!({"token": "abc123"})).unwrap(),
    )
    .unwrap();

    let state_dir = dir.path().join("state");
    let config_out = dir.path().join("custom-config.json");
    let outcome = provider_onboarding::onboard(OnboardRequest {
        pack_path,
        provider_type: None,
        config_path: Some(config_path),
        strict: true,
        instance_id: Some("primary".into()),
        state_dir: Some(state_dir.clone()),
        allow_remote_in_offline: true,
        greentic_config: None,
        config_out: Some(config_out.clone()),
    })
    .expect("onboard succeeds");

    assert_eq!(config_out, outcome.config_path);
    let stored = fs::read_to_string(&config_out).unwrap();
    assert!(stored.contains("\"provider_type\""));
    assert!(stored.contains("dev.greentic.provider"));
}
