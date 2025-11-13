use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use tracing::{info, info_span};

use crate::config::{Action, DeployerConfig};
use crate::error::Result;
use crate::iac::{
    DefaultIaCCommandRunner, IaCCommandRunner, IaCTool, dry_run_commands, run_iac_destroy,
    run_iac_plan_apply,
};
use crate::pack_introspect;
use crate::plan::SecretSpec;
use crate::providers::{ProviderArtifacts, ResolvedSecret, create_backend};
use crate::secrets::SecretsContext;
use crate::telemetry;
use greentic_telemetry::{TelemetryCtx, set_current_telemetry_ctx};

pub async fn run(config: DeployerConfig) -> Result<()> {
    run_with_runner(config, &DefaultIaCCommandRunner).await
}

pub async fn run_with_runner(config: DeployerConfig, runner: &dyn IaCCommandRunner) -> Result<()> {
    telemetry::init(&config)?;

    let plan = {
        let span = stage_span("plan", &config);
        let _enter = span.enter();
        install_telemetry_context("plan", &config);
        pack_introspect::build_plan(&config)?
    };
    info!("built deployment plan: {}", plan.summary());

    let backend = create_backend(config.provider, &config, &plan)?;
    let artifacts = backend.plan().await?;
    write_artifacts(&config, &artifacts)?;

    let deploy_dir = PathBuf::from("deploy")
        .join(artifacts.provider.as_str())
        .join(&config.tenant)
        .join(&config.environment);

    println!("{}", plan.summary());
    println!(
        "Artifacts stored under deploy/{}/{}/{}",
        artifacts.provider.as_str(),
        config.tenant,
        config.environment
    );

    let secrets_client = SecretsContext::discover(&config).await?;

    match config.action {
        Action::Plan => {
            if config.preview {
                println!("Preview mode: nothing was applied.");
            }
            Ok(())
        }
        Action::Apply => {
            if config.preview {
                println!("Preview mode: skipping apply.");
                return Ok(());
            }
            if config.dry_run {
                print_dry_run_commands(config.iac_tool, false, &deploy_dir);
                return Ok(());
            }
            if config.yes || confirm_or_cancel("apply")? {
                let span = stage_span("apply", &config);
                let _enter = span.enter();
                install_telemetry_context("apply", &config);
                let resolved = resolve_secrets(&secrets_client, &plan.secrets).await?;
                secrets_client
                    .push_to_provider(config.provider, &resolved)
                    .await?;
                backend.apply(&artifacts, &resolved).await?;
                run_iac_plan_apply(runner, config.iac_tool, &deploy_dir)?;
            }
            Ok(())
        }
        Action::Destroy => {
            if config.preview {
                println!("Preview mode: skipping destroy.");
                return Ok(());
            }
            if config.dry_run {
                print_dry_run_commands(config.iac_tool, true, &deploy_dir);
                return Ok(());
            }
            if config.yes || confirm_or_cancel("destroy")? {
                let span = stage_span("destroy", &config);
                let _enter = span.enter();
                install_telemetry_context("destroy", &config);
                let resolved = resolve_secrets(&secrets_client, &plan.secrets).await?;
                secrets_client
                    .push_to_provider(config.provider, &resolved)
                    .await?;
                backend.destroy(&artifacts, &resolved).await?;
                run_iac_destroy(runner, config.iac_tool, &deploy_dir)?;
            }
            Ok(())
        }
    }
}

fn confirm_or_cancel(action: &str) -> Result<bool> {
    print!("Confirm {}? [y/N]: ", action);
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    let response = buf.trim().to_ascii_lowercase();
    if response == "y" || response == "yes" {
        Ok(true)
    } else {
        println!("Skipping {}.", action);
        Ok(false)
    }
}

fn write_artifacts(config: &DeployerConfig, artifacts: &ProviderArtifacts) -> Result<()> {
    let base = PathBuf::from("deploy")
        .join(artifacts.provider.as_str())
        .join(&config.tenant)
        .join(&config.environment);
    fs::create_dir_all(&base)?;

    for file in &artifacts.files {
        let target = base.join(&file.relative_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&target, &file.contents)?;
    }

    Ok(())
}

async fn resolve_secrets(
    client: &SecretsContext,
    specs: &[SecretSpec],
) -> Result<Vec<ResolvedSecret>> {
    let mut resolved = Vec::new();
    for spec in specs {
        let provider_path = client.logical_to_provider_path(&spec.name);
        let value = client.resolve(&spec.name).await?;
        info!(
            "resolved secret {} -> {} ({} bytes)",
            spec.name,
            provider_path,
            value.len()
        );
        resolved.push(ResolvedSecret {
            spec: spec.clone(),
            value,
            provider_path,
        });
    }
    Ok(resolved)
}

fn stage_span(stage: &str, config: &DeployerConfig) -> tracing::Span {
    let span = info_span!(
        "deployment",
        stage,
        tenant = %config.tenant,
        environment = %config.environment,
        provider = %config.provider.as_str()
    );
    span.record("greentic.deployer.provider", &config.provider.as_str());
    span.record("greentic.deployer.tenant", &config.tenant.as_str());
    span.record(
        "greentic.deployer.environment",
        &config.environment.as_str(),
    );
    span
}

fn install_telemetry_context(stage: &str, config: &DeployerConfig) {
    let session = format!("{stage}/{env}", stage = stage, env = config.environment);
    let ctx = TelemetryCtx::new(config.tenant.clone())
        .with_provider(config.provider.as_str())
        .with_session(session);
    set_current_telemetry_ctx(ctx);
}

fn print_dry_run_commands(tool: IaCTool, destroy: bool, deploy_dir: &Path) {
    println!(
        "Dry run: IaC commands for {} would execute inside {}",
        tool,
        deploy_dir.display()
    );
    for command in dry_run_commands(destroy) {
        println!("{} {}", tool.binary_name(), command.join(" "));
    }
}
