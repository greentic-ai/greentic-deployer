use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use greentic_deployer::error::DeployerError;
use greentic_deployer::plan::DeploymentPlan;
use greentic_deployer::{
    apply,
    config::{Action, DeployerConfig, Provider},
    iac::{IaCCommandRunner, IaCTool},
    pack_introspect,
    secrets::{clear_test_secrets, register_test_secret},
};

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

async fn seed_secrets(config: &DeployerConfig, plan: &DeploymentPlan) -> Result<(), DeployerError> {
    clear_test_secrets();
    for spec in &plan.secrets {
        register_test_secret(
            &config.environment,
            &config.tenant,
            &spec.name,
            &format!("test-value-{}", spec.name),
        );
    }
    Ok(())
}

fn cleanup_deploy() {
    let _ = fs::remove_dir_all("deploy");
}

#[tokio::test]
async fn acme_pack_end_to_end() -> Result<(), DeployerError> {
    cleanup_deploy();

    let apply_config = DeployerConfig {
        action: Action::Apply,
        provider: Provider::Aws,
        tenant: "acme".into(),
        environment: "staging".into(),
        pack_path: "examples/acme-pack".into(),
        yes: true,
        preview: false,
        dry_run: false,
        iac_tool: IaCTool::Terraform,
    };

    let plan = pack_introspect::build_plan(&apply_config)?;
    seed_secrets(&apply_config, &plan).await?;
    let runner = TestRunner::new();
    apply::run_with_runner(apply_config.clone(), &runner).await?;
    assert!(Path::new("deploy/aws/acme/staging/apply-manifest.json").exists());
    assert_eq!(runner.calls().len(), 3);

    let destroy_config = DeployerConfig {
        action: Action::Destroy,
        ..apply_config
    };

    apply::run_with_runner(destroy_config, &runner).await?;
    assert!(Path::new("deploy/aws/acme/staging/destroy-manifest.json").exists());
    assert_eq!(runner.calls().len(), 5);

    cleanup_deploy();
    Ok(())
}
