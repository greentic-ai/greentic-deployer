use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use greentic_deployer::{
    apply,
    config::{Action, DeployerConfig, Provider},
    error::DeployerError,
    iac::{IaCCommandRunner, IaCTool},
    pack_introspect,
    plan::PlanContext,
    secrets::{clear_test_secrets, register_test_secret},
};
use std::process::Command;

struct TestRunner {
    calls: Arc<Mutex<Vec<Vec<String>>>>,
}

impl TestRunner {
    fn new() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn calls(&self) -> Vec<Vec<String>> {
        self.calls.lock().unwrap().clone()
    }
}

impl IaCCommandRunner for TestRunner {
    fn run(&self, _tool: IaCTool, _dir: &Path, args: &[&str]) -> Result<(), DeployerError> {
        let mut guard = self.calls.lock().unwrap();
        guard.push(args.iter().map(|s| s.to_string()).collect());
        Ok(())
    }
}

async fn seed_secrets(config: &DeployerConfig, plan: &PlanContext) -> Result<(), DeployerError> {
    clear_test_secrets();
    for spec in &plan.secrets {
        register_test_secret(
            &config.environment,
            &config.tenant,
            &spec.key,
            &format!("test-value-{}", spec.key),
        );
    }
    Ok(())
}

fn cleanup_deploy() {
    let _ = fs::remove_dir_all("deploy");
}

async fn run_pack_flow(
    provider: Provider,
    tenant: &str,
    pack_path: &str,
) -> Result<(), DeployerError> {
    cleanup_deploy();

    let apply_config = DeployerConfig {
        action: Action::Apply,
        provider,
        strategy: "iac-only".into(),
        tenant: tenant.into(),
        environment: "staging".into(),
        pack_path: pack_path.into(),
        yes: true,
        preview: false,
        dry_run: false,
        iac_tool: IaCTool::Terraform,
    };

    let plan = pack_introspect::build_plan(&apply_config)?;
    seed_secrets(&apply_config, &plan).await?;
    let runner = TestRunner::new();

    let deploy_root = Path::new("deploy")
        .join(apply_config.provider.as_str())
        .join(&apply_config.tenant)
        .join(&apply_config.environment);

    apply::run_with_runner(apply_config.clone(), &runner).await?;
    let apply_manifest_path = deploy_root.join("apply-manifest.json");
    assert!(apply_manifest_path.exists());
    let manifest_json = fs::read_to_string(&apply_manifest_path).expect("apply manifest readable");
    let manifest: serde_json::Value =
        serde_json::from_str(&manifest_json).expect("apply manifest parses");
    let secrets = manifest
        .get("secrets")
        .and_then(|value| value.as_array())
        .expect("secrets array present");
    assert!(!secrets.is_empty(), "expected secrets recorded in manifest");
    let oauth_clients = manifest
        .get("oauth_clients")
        .and_then(|value| value.as_array())
        .expect("oauth_clients array present");
    assert!(
        !oauth_clients.is_empty(),
        "expected oauth clients recorded in apply manifest"
    );
    assert_eq!(runner.calls().len(), 3);

    let destroy_config = DeployerConfig {
        action: Action::Destroy,
        ..apply_config
    };

    apply::run_with_runner(destroy_config, &runner).await?;
    assert!(deploy_root.join("destroy-manifest.json").exists());
    assert_eq!(runner.calls().len(), 5);

    cleanup_deploy();
    Ok(())
}

#[tokio::test]
async fn packs_end_to_end_all_providers() -> Result<(), DeployerError> {
    let packs = [
        ("acme", "examples/acme-pack"),
        ("acmeplus", "examples/acme-plus-pack"),
    ];
    for provider in [Provider::Aws, Provider::Azure, Provider::Gcp] {
        for (tenant, pack) in packs {
            run_pack_flow(provider, tenant, pack).await?;
        }
    }
    Ok(())
}

#[test]
fn dry_run_cli_smoke_test() {
    let binary = env!("CARGO_BIN_EXE_greentic-deployer");
    let packs = [
        ("acme", "examples/acme-pack"),
        ("acmeplus", "examples/acme-plus-pack"),
    ];
    for provider in ["aws", "azure", "gcp"] {
        for (tenant, pack) in packs {
            for action in ["apply", "destroy"] {
                cleanup_deploy();
                let status = Command::new(binary)
                    .args([
                        action,
                        "--provider",
                        provider,
                        "--tenant",
                        tenant,
                        "--environment",
                        "staging",
                        "--pack",
                        pack,
                        "--dry-run",
                        "--yes",
                    ])
                    .status()
                    .expect("spawn greentic-deployer");
                assert!(
                    status.success(),
                    "dry-run {action} for {provider} ({tenant}) failed with status {status:?}"
                );
            }
        }
    }
    cleanup_deploy();
}
