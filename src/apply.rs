use std::fs;
use std::io::{self, Write};
use std::path::Path;

use tracing::{info, info_span};

use crate::config::{Action, DeployerConfig, OutputFormat};
use crate::deployment::{DeploymentTarget, execute_deployment_pack, resolve_dispatch};
use crate::error::{DeployerError, Result};
use crate::iac::{
    DefaultIaCCommandRunner, IaCCommandRunner, IaCTool, dry_run_commands, run_iac_destroy,
    run_iac_plan_apply,
};
use crate::pack_introspect;
use crate::plan::{PlanContext, requirement_scope};
use crate::providers::{ProviderArtifacts, ResolvedSecret, create_backend};
use crate::secrets::{SecretFetchOutcome, SecretsContext};
use crate::telemetry;
use greentic_telemetry::{TelemetryCtx, set_current_telemetry_ctx};
use greentic_types::secrets::SecretRequirement;
use serde_json;
use serde_yaml_bw as serde_yaml;

pub async fn run(config: DeployerConfig) -> Result<()> {
    run_with_runner(config, &DefaultIaCCommandRunner).await
}

/// Entry point used by the CLI: builds the plan from the pack and forwards to [`run_with_plan`].
pub async fn run_with_runner(config: DeployerConfig, runner: &dyn IaCCommandRunner) -> Result<()> {
    telemetry::init(&config)?;
    let plan = {
        let span = stage_span("plan", &config);
        let _enter = span.enter();
        install_telemetry_context("plan", &config);
        pack_introspect::build_plan(&config)?
    };
    run_with_plan(config, plan, runner).await
}

/// Executes a deployment given an already constructed [`PlanContext`].
///
/// This is the entry point greentic-runner/control planes should invoke after producing the plan.
/// Callers are expected to have initialised telemetry already (e.g. via `telemetry::init`).
pub async fn run_with_plan(
    config: DeployerConfig,
    plan: PlanContext,
    runner: &dyn IaCCommandRunner,
) -> Result<()> {
    info!("built deployment plan: {}", plan.summary());

    let plan_target = DeploymentTarget {
        provider: plan.deployment.provider.clone(),
        strategy: plan.deployment.strategy.clone(),
    };
    if plan_target.provider != config.provider.as_str() || plan_target.strategy != config.strategy {
        info!(
            "deployment plan target provider={} strategy={} (cli requested {}::{})",
            plan_target.provider,
            plan_target.strategy,
            config.provider.as_str(),
            config.strategy
        );
    }
    let dispatch = resolve_dispatch(&plan_target)?;
    info!(
        "resolved deployment pack {}::{} for provider={} strategy={}",
        dispatch.pack_id, dispatch.flow_id, plan_target.provider, plan_target.strategy
    );
    if execute_deployment_pack(&config, &plan, &dispatch).await? {
        info!("deployment plan executed via deployment pack; skipping legacy provider backend");
        return Ok(());
    }

    let backend = create_backend(config.provider, &config, &plan)?;
    let artifacts = backend.plan().await?;
    write_artifacts(&config, &artifacts)?;

    let deploy_dir = config.provider_output_dir();

    let render_text = config.action != Action::Plan || matches!(config.output, OutputFormat::Text);
    if render_text {
        println!("{}", plan.summary());
        println!("Artifacts stored under {}", deploy_dir.display());
    }

    let secrets_client = SecretsContext::discover(&config).await?;

    match config.action {
        Action::Plan => {
            render_plan_output(&config, &plan)?;
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
                let resolved =
                    resolve_secrets(&secrets_client, &plan.secrets, &plan, &config).await?;
                secrets_client.push_to_provider(&resolved).await?;
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
                let resolved =
                    resolve_secrets(&secrets_client, &plan.secrets, &plan, &config).await?;
                secrets_client.push_to_provider(&resolved).await?;
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
    let base = config.provider_output_dir();
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
    specs: &[SecretRequirement],
    plan: &PlanContext,
    config: &DeployerConfig,
) -> Result<Vec<ResolvedSecret>> {
    let mut resolved = Vec::new();
    let mut missing = Vec::new();

    for requirement in specs {
        match client.fetch(requirement).await {
            SecretFetchOutcome::Present {
                requirement,
                provider_path,
                value,
            } => {
                let scope =
                    requirement_scope(&requirement, &plan.plan.environment, &plan.plan.tenant);
                info!(
                    "resolved secret {} (env={}, tenant={}, team={:?}) -> {} bytes",
                    requirement.key.as_str(),
                    scope.env,
                    scope.tenant,
                    scope.team,
                    value.len()
                );
                resolved.push(ResolvedSecret {
                    requirement,
                    value,
                    provider_path,
                });
            }
            SecretFetchOutcome::Missing {
                requirement,
                provider_path,
                error,
            } => {
                if requirement.required {
                    missing.push((requirement, provider_path, error));
                } else {
                    let scope =
                        requirement_scope(&requirement, &plan.plan.environment, &plan.plan.tenant);
                    info!(
                        "optional secret {} missing in env={} tenant={}; continuing",
                        requirement.key.as_str(),
                        scope.env,
                        scope.tenant
                    );
                }
            }
        }
    }

    if !missing.is_empty() {
        let missing_keys: Vec<String> = missing
            .iter()
            .map(|(req, path, err)| {
                let scope = requirement_scope(req, &plan.plan.environment, &plan.plan.tenant);
                format!(
                    "{} (env={}, tenant={}, team={:?}, path={}, error={})",
                    req.key.as_str(),
                    scope.env,
                    scope.tenant,
                    scope.team,
                    path,
                    err
                )
            })
            .collect();
        let hint = format!(
            "greentic-secrets init --pack {}",
            config.pack_path.display()
        );
        return Err(DeployerError::MissingSecrets {
            pack_id: plan.plan.pack_id.clone(),
            pack_version: plan.plan.pack_version.to_string(),
            missing: missing_keys,
            hint,
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
    span.record("greentic.deployer.provider", config.provider.as_str());
    span.record("greentic.deployer.tenant", config.tenant.as_str());
    span.record("greentic.deployer.environment", config.environment.as_str());
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

fn render_plan_output(config: &DeployerConfig, plan: &PlanContext) -> Result<()> {
    match config.output {
        OutputFormat::Text => {
            print_component_summary(plan);
        }
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(plan)
                .map_err(|err| DeployerError::Other(err.to_string()))?;
            println!("{json}");
        }
        OutputFormat::Yaml => {
            let yaml =
                serde_yaml::to_string(plan).map_err(|err| DeployerError::Other(err.to_string()))?;
            println!("{yaml}");
        }
    }
    Ok(())
}

fn print_component_summary(plan: &PlanContext) {
    if plan.components.is_empty() {
        println!("No component role/profile mappings available.");
        return;
    }

    println!("Component mappings for target {}:", plan.target.as_str());
    for component in &plan.components {
        println!(
            "- {}: role={} profile={} infra={}",
            component.id,
            component.role.as_str(),
            component.profile.as_str(),
            component.infra.summary
        );
        if !component.infra.resources.is_empty() {
            println!("  resources: {}", component.infra.resources.join(", "));
        }
        if let Some(inference) = &component.inference {
            if !inference.warnings.is_empty() {
                for warning in &inference.warnings {
                    println!("  warning: {warning}");
                }
            } else {
                println!("  info: {}", inference.source);
            }
        }
    }
}
