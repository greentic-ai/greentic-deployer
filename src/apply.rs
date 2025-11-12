use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use tracing::{info, info_span};

use crate::config::{Action, DeployerConfig};
use crate::error::Result;
use crate::pack_introspect;
use crate::plan::SecretSpec;
use crate::providers::{ProviderArtifacts, create_backend};
use crate::secrets::SecretsAdapter;
use crate::telemetry;
use greentic_telemetry::{TelemetryCtx, set_current_telemetry_ctx};

pub async fn run(config: DeployerConfig) -> Result<()> {
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

    println!("{}", plan.summary());
    println!(
        "Artifacts stored under deploy/{}/{}/{}",
        artifacts.provider.as_str(),
        config.tenant,
        config.environment
    );

    let secrets_client = SecretsAdapter::discover(&config).await?;

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
            if config.yes || confirm_or_cancel("apply")? {
                let span = stage_span("apply", &config);
                let _enter = span.enter();
                install_telemetry_context("apply", &config);
                ensure_secrets(&secrets_client, &plan.secrets).await?;
                backend.apply(&artifacts).await?;
            }
            Ok(())
        }
        Action::Destroy => {
            if config.preview {
                println!("Preview mode: skipping destroy.");
                return Ok(());
            }
            if config.yes || confirm_or_cancel("destroy")? {
                let span = stage_span("destroy", &config);
                let _enter = span.enter();
                install_telemetry_context("destroy", &config);
                ensure_secrets(&secrets_client, &plan.secrets).await?;
                backend.destroy(&artifacts).await?;
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

    for artifact in &artifacts.artifacts {
        let target = base.join(&artifact.path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&target, &artifact.contents)?;
    }

    Ok(())
}

async fn ensure_secrets(client: &SecretsAdapter, specs: &[SecretSpec]) -> Result<()> {
    for spec in specs {
        match client.read(&spec.name).await {
            Ok(value) => {
                info!("resolved secret {} ({} bytes)", spec.name, value.len());
            }
            Err(err) => {
                return Err(err);
            }
        }
    }
    Ok(())
}

fn stage_span(stage: &str, config: &DeployerConfig) -> tracing::Span {
    info_span!(
        "deployment",
        stage,
        tenant = %config.tenant,
        environment = %config.environment,
        provider = %config.provider.as_str()
    )
}

fn install_telemetry_context(stage: &str, config: &DeployerConfig) {
    let session = format!("{stage}/{env}", stage = stage, env = config.environment);
    let ctx = TelemetryCtx::new(config.tenant.clone())
        .with_provider(config.provider.as_str())
        .with_session(session);
    set_current_telemetry_ctx(ctx);
}
