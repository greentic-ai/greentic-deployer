use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use greentic_deployer::platform::load_platform_pack;
use greentic_types::cbor::encode_pack_manifest;
use greentic_types::component::{ComponentCapabilities, ComponentManifest, ComponentProfiles};
use greentic_types::flow::FlowKind;
use greentic_types::pack_manifest::{PackKind, PackManifest};
use greentic_types::{ComponentId, PackId};
use semver::Version;
use tar::Builder;
use tempfile::tempdir;

use greentic_deployer::platform::{VerificationPolicy, verify_platform_pack};
use greentic_types::pack::{Signature, SignatureAlgorithm};

fn write_gtpack(manifest: &PackManifest, path: &PathBuf) {
    let encoded = encode_pack_manifest(manifest).expect("encode manifest");
    let mut builder = Builder::new(Vec::new());

    let mut header = tar::Header::new_gnu();
    header.set_size(encoded.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(&mut header, "manifest.cbor", encoded.as_slice())
        .expect("append manifest");

    let bytes = builder.into_inner().expect("tar bytes");
    let mut file = File::create(path).expect("create gtpack");
    file.write_all(&bytes).expect("write gtpack");
}

fn sample_manifest() -> PackManifest {
    PackManifest {
        schema_version: "pack-v1".to_string(),
        pack_id: PackId::try_from("dev.greentic.platform").unwrap(),
        version: Version::new(0, 1, 0),
        kind: PackKind::Application,
        publisher: "greentic".to_string(),
        components: vec![ComponentManifest {
            id: ComponentId::try_from("dev.greentic.platform.installer").unwrap(),
            version: Version::new(0, 1, 0),
            supports: vec![FlowKind::Messaging],
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
        secret_requirements: Vec::new(),
        signatures: Default::default(),
        bootstrap: None,
        extensions: None,
    }
}

#[test]
fn loads_platform_pack_from_gtpack() {
    let manifest = sample_manifest();
    let dir = tempdir().expect("temp dir");
    let path = dir.path().join("platform.gtpack");
    write_gtpack(&manifest, &path);

    let info = load_platform_pack(&path).expect("load platform pack");
    assert_eq!(info.manifest.pack_id, manifest.pack_id);
    assert_eq!(info.manifest.version, manifest.version);
    assert!(info.digest.is_some(), "digest should be computed");
}

#[test]
fn errors_on_missing_pack() {
    let err = load_platform_pack(PathBuf::from("missing.gtpack").as_path()).unwrap_err();
    assert!(
        format!("{err}").contains("does not exist"),
        "expected missing error, got {err}"
    );
}

#[test]
fn missing_signatures_warns_when_non_strict() {
    let manifest = sample_manifest();
    let dir = tempdir().expect("temp dir");
    let path = dir.path().join("platform.gtpack");
    write_gtpack(&manifest, &path);

    let info = load_platform_pack(&path).expect("load platform pack");
    let outcome = verify_platform_pack(
        &info,
        VerificationPolicy {
            verify: true,
            strict: false,
        },
    )
    .expect("non-strict verification should pass");
    assert!(
        outcome
            .warnings
            .iter()
            .any(|w| w.contains("missing signatures")),
        "expected missing signature warning"
    );
}

#[test]
fn missing_signatures_fails_when_strict() {
    let manifest = sample_manifest();
    let dir = tempdir().expect("temp dir");
    let path = dir.path().join("platform.gtpack");
    write_gtpack(&manifest, &path);

    let info = load_platform_pack(&path).expect("load platform pack");
    let err = verify_platform_pack(
        &info,
        VerificationPolicy {
            verify: true,
            strict: true,
        },
    )
    .unwrap_err();
    assert!(
        format!("{err}").contains("missing signatures"),
        "expected strict failure on missing signatures"
    );
}

#[test]
fn invalid_signature_fails() {
    let mut manifest = sample_manifest();
    manifest.signatures.signatures.push(Signature {
        key_id: "test-key".to_string(),
        algorithm: SignatureAlgorithm::Ed25519,
        signature: Vec::new(), // invalid empty signature
    });

    let dir = tempdir().expect("temp dir");
    let path = dir.path().join("platform.gtpack");
    write_gtpack(&manifest, &path);

    let info = load_platform_pack(&path).expect("load platform pack");
    let err = verify_platform_pack(
        &info,
        VerificationPolicy {
            verify: true,
            strict: true,
        },
    )
    .unwrap_err();
    assert!(
        format!("{err}").contains("invalid signature"),
        "expected invalid signature failure"
    );
}
