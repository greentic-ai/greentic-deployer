use std::fs;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use greentic_config::{ConfigFileFormat, ConfigLayer, ConfigResolver, ProvenanceMap};
use greentic_config_types::{GreenticConfig, PathsConfig, TelemetryConfig};
use greentic_types::ConnectionKind;
use greentic_types::pack::PackRef;
use semver::Version;
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

    /// Environment name (defaults to greentic-config environment).
    #[arg(long)]
    pub environment: Option<String>,

    /// Path to a .greentic-pack archive or a pack directory.
    #[arg(long)]
    pub pack: PathBuf,

    /// Optional pack identifier to resolve from a distributor/registry.
    #[arg(long)]
    pub pack_id: Option<String>,

    /// Pack version to resolve from a distributor/registry (requires --pack-id).
    #[arg(long)]
    pub pack_version: Option<String>,

    /// Pack digest to resolve from a distributor/registry (requires --pack-id).
    #[arg(long)]
    pub pack_digest: Option<String>,

    /// Optional distributor base URL for registry resolution.
    #[arg(long)]
    pub distributor_url: Option<String>,

    /// Optional auth token for the distributor.
    #[arg(long)]
    pub distributor_token: Option<String>,

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
    long_about = "Choose Terraform or OpenTofu via --iac-tool, or rely on PATH auto-detection (tofu takes precedence). Apply/destroy commands run terraform/tofu init/plan/apply or init/destroy inside deploy/<provider>/<tenant>/<env>."
)]
pub struct CliArgs {
    #[command(flatten)]
    pub global: GlobalArgs,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Args, Default)]
pub struct GlobalArgs {
    /// Optional explicit config path (overrides project/user discovery).
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    /// Print resolved configuration (with provenance) and exit.
    #[arg(long, default_value_t = false, global = true)]
    pub explain_config: bool,

    /// Print resolved configuration in JSON form (with provenance) and exit.
    #[arg(long, default_value_t = false, global = true)]
    pub explain_config_json: bool,

    /// Allow using remote endpoints even when ConnectionKind is Offline.
    #[arg(long, default_value_t = false, global = true)]
    pub allow_remote_in_offline: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Build deployment artifacts without applying them.
    Plan(ActionArgs),
    /// Apply a planned deployment.
    Apply(ActionArgs),
    /// Destroy resources created by apply.
    Destroy(ActionArgs),
    /// Platform bootstrap commands (install/upgrade/status).
    Platform(PlatformArgs),
    /// Provider onboarding commands.
    Provider {
        #[command(subcommand)]
        command: ProviderArgs,
    },
}

#[derive(Subcommand, Debug)]
pub enum PlatformCommand {
    /// Install the Greentic platform from a .gtpack.
    Install(PlatformActionArgs),
    /// Upgrade the Greentic platform from a .gtpack.
    Upgrade(PlatformActionArgs),
    /// Show bootstrap state/status.
    Status,
}

#[derive(Debug, Args)]
pub struct PlatformActionArgs {
    /// Path to a platform .gtpack archive (or oci:// reference when network is allowed).
    #[arg(long)]
    pub pack: String,
    /// Enable signature verification (warnings on missing signatures).
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub verify: bool,
    /// Fail when signatures are missing or invalid.
    #[arg(long, default_value_t = false)]
    pub strict_verify: bool,
}

#[derive(Args, Debug)]
pub struct PlatformArgs {
    #[command(subcommand)]
    pub command: PlatformCommand,
    /// Path to bootstrap state file (defaults to /var/lib/greentic/bootstrap/state.json).
    #[arg(
        long,
        global = true,
        default_value = "/var/lib/greentic/bootstrap/state.json",
        env = "GREENTIC_BOOTSTRAP_STATE"
    )]
    pub bootstrap_state: PathBuf,
    /// Bootstrap state backend (file|k8s).
    #[arg(long, global = true, value_enum, default_value = "file")]
    pub bootstrap_state_backend: BootstrapStateBackend,
    /// Kubernetes namespace for bootstrap state (when using k8s backend).
    #[arg(long, global = true, default_value = "greentic-system")]
    pub k8s_namespace: String,
    /// Kubernetes ConfigMap/Secret name for bootstrap state (when using k8s backend).
    #[arg(long, global = true, default_value = "greentic-bootstrap")]
    pub k8s_state_name: String,
    /// Interaction mode for bootstrap (cli|json|auto).
    #[arg(long, global = true, value_enum, default_value = "auto")]
    pub interaction: InteractionMode,
    /// Explicitly allow listener-based adapters (HTTP/MQTT); defaults to off.
    #[arg(long, global = true, default_value_t = false)]
    pub allow_listeners: bool,
    /// Allow network access for interaction adapters (off by default).
    #[arg(long, global = true, default_value_t = false)]
    pub allow_network: bool,
    /// Comma-separated allowlist for outbound network targets (domains or CIDRs).
    #[arg(long, global = true)]
    pub net_allowlist: Option<String>,
    /// Optional bind address for listener adapters.
    #[arg(long, global = true)]
    pub bind: Option<String>,
    /// Force offline-only bootstrap (no network/remote fetches).
    #[arg(long, global = true, default_value_t = false)]
    pub offline_only: bool,
    /// Secrets backend URI for bootstrap secret writes (e.g., file:/var/lib/greentic/secrets.db).
    #[arg(
        long,
        global = true,
        default_value = "file:/var/lib/greentic/secrets.db",
        env = "GREENTIC_SECRETS_BACKEND"
    )]
    pub secrets_backend: String,
    /// Answers JSON for non-interactive bootstrap (use @- for stdin).
    #[arg(long, global = true)]
    pub answers: Option<String>,
    /// Output file to write redacted bootstrap output JSON.
    #[arg(long, global = true)]
    pub output: Option<PathBuf>,
    /// Path to write applied config_patch JSON (defaults beside bootstrap state).
    #[arg(long, global = true)]
    pub config_out: Option<PathBuf>,
    /// Interaction timeout in seconds (applies to HTTP/MQTT adapters).
    #[arg(long, global = true, default_value_t = 30)]
    pub interaction_timeout: u64,
}

#[derive(Subcommand, Debug)]
pub enum ProviderArgs {
    /// Onboard a provider from a pack + provider extension metadata.
    Onboard(ProviderOnboardArgs),
}

#[derive(Debug, Args)]
pub struct ProviderOnboardArgs {
    /// Path to a provider pack (.gtpack) or unpacked directory.
    #[arg(long)]
    pub pack: PathBuf,
    /// Provider type to select when multiple providers are present.
    #[arg(long)]
    pub provider_type: Option<String>,
    /// Non-interactive config JSON path (skips prompts).
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Optional output path for persisted provider config (defaults to state dir).
    #[arg(long)]
    pub config_out: Option<PathBuf>,
    /// Fail when remote schemas/extension payloads are unpinned.
    #[arg(long, default_value_t = false)]
    pub strict: bool,
    /// Provider instance identifier to persist (defaults to provider_type).
    #[arg(long)]
    pub instance_id: Option<String>,
    /// Override state directory for persisted provider configs.
    #[arg(long)]
    pub state_dir: Option<PathBuf>,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionMode {
    Auto,
    Cli,
    Json,
    Http,
    Mqtt,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapStateBackend {
    File,
    K8s,
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
    pub pack_ref: Option<PackRef>,
    pub distributor_url: Option<String>,
    pub distributor_token: Option<String>,
    pub yes: bool,
    pub preview: bool,
    pub dry_run: bool,
    pub iac_tool: IaCTool,
    pub output: OutputFormat,
    pub greentic: GreenticConfig,
    pub provenance: ProvenanceMap,
    pub config_warnings: Vec<String>,
    pub explain_config: bool,
    pub explain_config_json: bool,
    pub allow_remote_in_offline: bool,
}

impl DeployerConfig {
    pub fn from_env_and_args(cli: CliArgs) -> Result<Self> {
        let (action, args) = match cli.command {
            Command::Plan(args) => (Action::Plan, args),
            Command::Apply(args) => (Action::Apply, args),
            Command::Destroy(args) => (Action::Destroy, args),
            Command::Platform(_) | Command::Provider { .. } => {
                return Err(DeployerError::Config(
                    "platform/provider commands do not use DeployerConfig".into(),
                ));
            }
        };

        let mut resolver = ConfigResolver::new();
        if let Some(layer) = load_explicit_config(cli.global.config.as_ref())? {
            resolver = resolver.with_cli_overrides(layer);
        }
        let resolved = resolver
            .load()
            .map_err(|err| DeployerError::Config(err.to_string()))?;
        let greentic = resolved.config;

        if !args.pack.exists() && args.pack_id.is_none() {
            return Err(DeployerError::Config(format!(
                "pack path {} does not exist (and no --pack-id provided)",
                args.pack.display()
            )));
        }

        let environment = env_id_to_string(
            args.environment
                .clone()
                .or_else(|| Some(greentic.environment.env_id.to_string())),
        )?;

        let iac_tool = resolve_iac_tool(args.iac_tool, None)?;
        let pack_ref = build_pack_ref(&args)?;

        let distributor_url = args.distributor_url;
        let distributor_token = args.distributor_token;

        validate_offline_policy(
            greentic.environment.connection.as_ref(),
            &pack_ref,
            distributor_url.as_deref(),
            cli.global.allow_remote_in_offline,
        )?;

        Ok(Self {
            action,
            provider: args.provider,
            strategy: args.strategy,
            tenant: args.tenant,
            environment,
            pack_path: args.pack,
            pack_ref,
            distributor_url,
            distributor_token,
            yes: args.yes,
            preview: args.preview,
            dry_run: args.dry_run,
            iac_tool,
            output: args.output,
            greentic,
            provenance: resolved.provenance,
            config_warnings: resolved.warnings,
            explain_config: cli.global.explain_config,
            explain_config_json: cli.global.explain_config_json,
            allow_remote_in_offline: cli.global.allow_remote_in_offline,
        })
    }

    pub fn deploy_base(&self) -> PathBuf {
        self.greentic.paths.state_dir.join("deploy")
    }

    pub fn provider_output_dir(&self) -> PathBuf {
        self.deploy_base()
            .join(self.provider.as_str())
            .join(&self.tenant)
            .join(&self.environment)
    }

    pub fn telemetry_config(&self) -> &TelemetryConfig {
        &self.greentic.telemetry
    }

    pub fn paths(&self) -> &PathsConfig {
        &self.greentic.paths
    }
}

fn load_explicit_config(path: Option<&PathBuf>) -> Result<Option<ConfigLayer>> {
    let Some(path) = path else {
        return Ok(None);
    };

    let contents = fs::read_to_string(path).map_err(|err| {
        DeployerError::Config(format!(
            "failed to read config file {}: {err}",
            path.display()
        ))
    })?;

    let format = match path.extension().and_then(|s| s.to_str()) {
        Some("json") => ConfigFileFormat::Json,
        _ => ConfigFileFormat::Toml,
    };

    let layer = match format {
        ConfigFileFormat::Toml => toml::from_str::<ConfigLayer>(&contents)
            .map_err(|err| format!("toml parse error: {err}")),
        ConfigFileFormat::Json => serde_json::from_str::<ConfigLayer>(&contents)
            .map_err(|err| format!("json parse error: {err}")),
    }
    .map_err(|err| {
        DeployerError::Config(format!("invalid config file {}: {err}", path.display()))
    })?;

    Ok(Some(layer))
}

fn build_pack_ref(args: &ActionArgs) -> Result<Option<PackRef>> {
    let Some(pack_id) = args.pack_id.as_ref() else {
        return Ok(None);
    };
    let version_str = args.pack_version.as_ref().ok_or_else(|| {
        DeployerError::Config("when using --pack-id you must set --pack-version".into())
    })?;
    let digest = args.pack_digest.as_ref().ok_or_else(|| {
        DeployerError::Config("when using --pack-id you must set --pack-digest".into())
    })?;
    let version = Version::parse(version_str).map_err(|err| {
        DeployerError::Config(format!("invalid pack version '{}': {}", version_str, err))
    })?;
    Ok(Some(PackRef::new(pack_id.clone(), version, digest.clone())))
}

fn env_id_to_string(env_id: Option<String>) -> Result<String> {
    Ok(env_id.unwrap_or_else(|| "dev".to_string()))
}

fn validate_offline_policy(
    connection: Option<&ConnectionKind>,
    pack_ref: &Option<PackRef>,
    distributor_url: Option<&str>,
    allow_remote_in_offline: bool,
) -> Result<()> {
    if matches!(connection, Some(ConnectionKind::Offline))
        && !allow_remote_in_offline
        && (pack_ref.is_some() || distributor_url.is_some())
    {
        return Err(DeployerError::OfflineDisallowed(
            "connection is Offline but remote pack/distributor requested; pass --allow-remote-in-offline to override".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

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

    fn write_config(dir: &Path) -> PathBuf {
        let cfg = r#"
[environment]
env_id = "prod"
connection = "offline"

[paths]
greentic_root = "."
state_dir = ".greentic/state"
cache_dir = ".greentic/cache"
logs_dir = ".greentic/logs"

[telemetry]
enabled = false

[network]
tls_mode = "system"

[secrets]
kind = "none"
"#;
        let path = dir.join("config.toml");
        fs::write(&path, cfg).expect("write config");
        path
    }

    #[test]
    fn defaults_to_dev_environment_when_missing() {
        if std::env::var("GREENTIC_ENV").is_ok() {
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

    #[test]
    fn rejects_pack_id_without_version_or_digest() {
        let mut args = base_args();
        args.push("--pack-id");
        args.push("dev.greentic.sample");
        let cli = CliArgs::parse_from(args);
        let err = DeployerConfig::from_env_and_args(cli).unwrap_err();
        assert!(
            format!("{err}").contains("--pack-version"),
            "expected version requirement error, got {err}"
        );
    }

    #[test]
    fn builds_pack_ref_when_provided() {
        let mut args = base_args();
        args.push("--pack-id");
        args.push("dev.greentic.sample");
        args.push("--pack-version");
        args.push("0.1.0");
        args.push("--pack-digest");
        args.push("sha256:deadbeef");
        let cli = CliArgs::parse_from(args);
        let config = DeployerConfig::from_env_and_args(cli).expect("config builds");
        let pack_ref = config.pack_ref.expect("pack_ref present");
        assert_eq!(pack_ref.oci_url, "dev.greentic.sample");
        assert_eq!(pack_ref.version.to_string(), "0.1.0");
        assert_eq!(pack_ref.digest, "sha256:deadbeef");
    }

    #[test]
    fn explicit_config_file_overrides_default_env() {
        let dir = tempdir().unwrap();
        let cfg_path = write_config(dir.path());

        let mut args = base_args();
        args.push("--config");
        args.push(cfg_path.to_str().unwrap());
        let cli = CliArgs::parse_from(args);
        let config = DeployerConfig::from_env_and_args(cli).expect("config builds");
        assert_eq!(config.greentic.environment.env_id.to_string(), "prod");
    }

    #[test]
    fn offline_connection_blocks_remote_pack_without_override() {
        let dir = tempdir().unwrap();
        let cfg_path = write_config(dir.path());

        let args = vec![
            "greentic-deployer",
            "plan",
            "--provider",
            "aws",
            "--tenant",
            "acme",
            "--pack",
            dir.path().to_str().unwrap(),
            "--pack-id",
            "dev.greentic.sample",
            "--pack-version",
            "0.1.0",
            "--pack-digest",
            "sha256:deadbeef",
            "--config",
            cfg_path.to_str().unwrap(),
            "--distributor-url",
            "https://distributor.greentic.ai",
        ];

        let cli = CliArgs::parse_from(&args);
        let err = DeployerConfig::from_env_and_args(cli).unwrap_err();
        assert!(
            format!("{err}").contains("Offline"),
            "expected offline validation error, got {err}"
        );
    }
}
