use clap::Parser;
use greentic_deployer::config::{CliArgs, Command, PlatformCommand};
use greentic_types::pack_manifest::PackManifest;
use serde_yaml_bw as serde_yaml;

fn minimal_manifest_yaml(with_bootstrap: bool) -> String {
    let bootstrap = if with_bootstrap {
        r#"
bootstrap:
  install_flow: platform_install
  upgrade_flow: platform_upgrade
  installer_component: installer
"#
    } else {
        ""
    };

    format!(
        r#"
schema_version: "pack-v1"
pack_id: "dev.greentic.platform"
version: "0.1.0"
kind: "application"
publisher: "greentic"
components: []
flows: []
dependencies: []
capabilities: []
secret_requirements: []
signatures: {{}}
{bootstrap}
"#
    )
}

#[test]
fn parses_manifest_without_bootstrap() {
    let yaml = minimal_manifest_yaml(false);
    let manifest: PackManifest =
        serde_yaml::from_str(&yaml).expect("parse manifest without bootstrap");
    assert!(
        manifest.bootstrap.is_none(),
        "bootstrap should default to None"
    );
}

#[test]
fn parses_manifest_with_bootstrap_and_roundtrips() {
    let yaml = minimal_manifest_yaml(true);
    let manifest: PackManifest =
        serde_yaml::from_str(&yaml).expect("parse manifest with bootstrap");
    let bootstrap = manifest.bootstrap.as_ref().expect("bootstrap present");
    assert_eq!(bootstrap.install_flow.as_deref(), Some("platform_install"));
    assert_eq!(bootstrap.upgrade_flow.as_deref(), Some("platform_upgrade"));
    assert_eq!(bootstrap.installer_component.as_deref(), Some("installer"));

    // Roundtrip via JSON to ensure serialization keeps the bootstrap block.
    let json = serde_json::to_string(&manifest).expect("serialize to json");
    let decoded: PackManifest = serde_json::from_str(&json).expect("deserialize from json");
    let roundtrip = decoded.bootstrap.as_ref().expect("bootstrap preserved");
    assert_eq!(roundtrip.install_flow.as_deref(), Some("platform_install"));
}

#[test]
fn cli_parses_platform_install_subcommand() {
    let cli = CliArgs::parse_from([
        "greentic-deployer",
        "platform",
        "install",
        "--pack",
        "platform.gtpack",
    ]);
    match cli.command {
        Command::Platform(platform) => match platform.command {
            PlatformCommand::Install(args) => {
                assert_eq!(args.pack, "platform.gtpack".to_string());
            }
            other => panic!("unexpected platform subcommand parsed: {other:?}"),
        },
        other => panic!("unexpected command parsed: {other:?}"),
    }
}
