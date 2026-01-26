use std::fs;
use std::path::{Path, PathBuf};

use clap::Parser;
use greentic_deployer::apply;
use greentic_deployer::config::{CliArgs, DeployerConfig};
use std::process::Command;
use tempfile::tempdir;

fn write_state_config(state_dir: &Path) -> PathBuf {
    let config = format!(
        r#"
[paths]
state_dir = "{state}"
"#,
        state = state_dir.display()
    );
    let file = state_dir.join("config.toml");
    fs::write(&file, config).expect("write config");
    file
}

fn unpack_provider_pack(provider: &str, dest_root: &Path) {
    let source = PathBuf::from(format!(
        "dist/greentic.demo.deploy.{provider}.gtpack",
        provider = provider
    ));
    fs::create_dir_all(dest_root).expect("create providers root");
    let target = dest_root.join(provider);
    fs::create_dir_all(&target).expect("create provider dir");
    let status = Command::new("unzip")
        .arg("-q")
        .arg("-d")
        .arg(&target)
        .arg(&source)
        .status()
        .expect("unzip provider pack");
    if !status.success() {
        panic!("unzip failed with {}", status);
    }
}

#[tokio::test]
async fn smoke_placeholder_local() {
    let temp = tempdir().expect("temp dir");
    let state_dir = temp.path().join("state");
    fs::create_dir_all(&state_dir).expect("create state dir");
    let config_file = write_state_config(&state_dir);

    let provider_root = temp.path().join("providers");
    unpack_provider_pack("local", &provider_root);

    let args = [
        "greentic-deployer",
        "--config",
        config_file.to_str().expect("config path"),
        "apply",
        "--provider",
        "local",
        "--strategy",
        "iac-only",
        "--tenant",
        "acme",
        "--environment",
        "dev",
        "--pack",
        "examples/acme-pack",
        "--providers-dir",
        provider_root.to_str().expect("provider path"),
        "--packs-dir",
        "dist",
        "--yes",
    ];

    let cli = CliArgs::parse_from(args);
    let config = DeployerConfig::from_env_and_args(cli).expect("build config");
    apply::run(config).await.expect("placeholder smoke");

    let deploy_dir = state_dir.join("deploy/local/acme/dev");
    assert!(deploy_dir.join("README.md").is_file());
    assert!(deploy_dir.join("local.sh").is_file());
    assert!(deploy_dir.join("._runner_cmd.txt").is_file());
    assert!(deploy_dir.join("._deployer_invocation.json").is_file());
}
