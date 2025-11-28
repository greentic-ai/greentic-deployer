use std::env;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

use crate::error::{DeployerError, Result};
use crate::iac::{IaCTool, IacToolArg, resolve_iac_tool};

/// Available CLI actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Plan,
    Apply,
    Destroy,
}

impl Action {
    pub fn as_str(&self) -> &'static str {
        match self {
            Action::Plan => "plan",
            Action::Apply => "apply",
            Action::Destroy => "destroy",
        }
    }
}

/// Supported deployment targets.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    Local,
    Aws,
    Azure,
    Gcp,
    K8s,
}

impl Provider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Provider::Local => "local",
            Provider::Aws => "aws",
            Provider::Azure => "azure",
            Provider::Gcp => "gcp",
            Provider::K8s => "k8s",
        }
    }
}

/// Output format for CLI commands.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
    Yaml,
}

/// Per-command configuration helpers.
#[derive(Debug, Args)]
pub struct ActionArgs {
    /// Deployment target (local|aws|azure|gcp|k8s).
    #[arg(long, value_enum)]
    pub provider: Provider,

    /// Deployment strategy identifier (e.g. serverless, vm, iac-only).
    #[arg(long, default_value = "iac-only")]
    pub strategy: String,

    /// Tenant identifier (e.g. acme).
    #[arg(long)]
    pub tenant: String,

    /// Environment name (defaults to $GREENTIC_ENV or \"dev\").
    #[arg(long)]
    pub environment: Option<String>,

    /// Path to a .greentic-pack archive or a pack directory.
    #[arg(long)]
    pub pack: PathBuf,

    /// Skip interactive confirmations (defaults to false).
    #[arg(long, default_value_t = false)]
    pub yes: bool,

    /// Treat the operation as a preview/dry-run.
    #[arg(long, default_value_t = false)]
    pub preview: bool,

    /// Generate IaC artifacts but do not execute them.
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,

    /// IaC tool to use (tf/terraform or tofu/opentofu).
    #[arg(long, value_enum)]
    pub iac_tool: Option<IacToolArg>,

    /// Output format for plan/rendering (text|json|yaml).
    #[arg(long, value_enum, default_value = "text")]
    pub output: OutputFormat,
}

/// Top-level CLI structure.
#[derive(Debug, Parser)]
#[command(
    name = "greentic-deployer",
    version,
    about = "Automated multi-cloud deployment engine for Greentic packs.",
    long_about = "Choose Terraform or OpenTofu via --iac-tool or GREENTIC_IAC_TOOL, or rely on PATH auto-detection (tofu takes precedence). Apply/destroy commands run terraform/tofu init/plan/apply or init/destroy inside deploy/<provider>/<tenant>/<env>."
)]
pub struct CliArgs {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Build deployment artifacts without applying them.
    Plan(ActionArgs),
    /// Apply a planned deployment.
    Apply(ActionArgs),
    /// Destroy resources created by apply.
    Destroy(ActionArgs),
}

/// Complete configuration used by the deployer.
#[derive(Debug, Clone)]
pub struct DeployerConfig {
    pub action: Action,
    pub provider: Provider,
    pub strategy: String,
    pub tenant: String,
    pub environment: String,
    pub pack_path: PathBuf,
    pub yes: bool,
    pub preview: bool,
    pub dry_run: bool,
    pub iac_tool: IaCTool,
    pub output: OutputFormat,
}

impl DeployerConfig {
    pub fn from_env_and_args(cli: CliArgs) -> Result<Self> {
        let (action, args) = match cli.command {
            Command::Plan(args) => (Action::Plan, args),
            Command::Apply(args) => (Action::Apply, args),
            Command::Destroy(args) => (Action::Destroy, args),
        };

        let environment = args
            .environment
            .or_else(|| env::var("GREENTIC_ENV").ok())
            .unwrap_or_else(|| "dev".to_string());

        if !args.pack.exists() {
            return Err(DeployerError::Config(format!(
                "pack path {} does not exist",
                args.pack.display()
            )));
        }

        let iac_tool = resolve_iac_tool(args.iac_tool, env::var("GREENTIC_IAC_TOOL").ok())?;

        Ok(Self {
            action,
            provider: args.provider,
            strategy: args.strategy,
            tenant: args.tenant,
            environment,
            pack_path: args.pack,
            yes: args.yes,
            preview: args.preview,
            dry_run: args.dry_run,
            iac_tool,
            output: args.output,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    fn base_args() -> Vec<&'static str> {
        vec![
            "greentic-deployer",
            "plan",
            "--provider",
            "aws",
            "--tenant",
            "acme",
            "--pack",
            "examples/acme-pack",
        ]
    }

    #[test]
    fn defaults_to_dev_environment_when_missing() {
        if env::var("GREENTIC_ENV").is_ok() {
            eprintln!("GREENTIC_ENV set; skipping default environment test");
            return;
        }

        let cli = CliArgs::parse_from(base_args());
        let config = DeployerConfig::from_env_and_args(cli).expect("config builds");
        assert_eq!(config.environment, "dev");
    }

    #[test]
    fn accepts_explicit_environment_flag() {
        let mut args = base_args();
        args.push("--environment");
        args.push("prod");
        let cli = CliArgs::parse_from(args);
        let config = DeployerConfig::from_env_and_args(cli).expect("config builds");
        assert_eq!(config.environment, "prod");
    }
}
