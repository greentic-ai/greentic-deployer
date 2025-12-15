use std::fs;
use std::path::PathBuf;

use greentic_types::cbor::encode_pack_manifest;
use greentic_types::component::{ComponentCapabilities, ComponentManifest, ComponentProfiles};
use greentic_types::pack_manifest::{PackKind, PackManifest};
use greentic_types::{ComponentId, PackId};
use semver::Version;

fn write_manifest(dir: &str, pack_id: &str) {
    let manifest = PackManifest {
        schema_version: "pack-v1".to_string(),
        pack_id: PackId::try_from(pack_id).unwrap(),
        version: Version::new(0, 1, 0),
        kind: PackKind::Application,
        publisher: "greentic".to_string(),
        components: vec![ComponentManifest {
            id: ComponentId::try_from(format!("{pack_id}.component")).unwrap(),
            version: Version::new(0, 1, 0),
            supports: Vec::new(),
            world: "greentic:test/world".to_string(),
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
        signatures: Default::default(),
    };
    let bytes = encode_pack_manifest(&manifest).expect("encode manifest");
    let path = PathBuf::from(dir).join("manifest.cbor");
    fs::write(&path, bytes).expect("write manifest");
    println!("wrote {}", path.display());
}

fn main() {
    write_manifest("examples/acme-pack", "greentic.acme");
    write_manifest("examples/acme-plus-pack", "greentic.acme.plus");
}
