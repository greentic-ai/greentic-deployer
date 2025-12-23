use clap::Parser;
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use greentic_deployer::{
    apply,
    bootstrap::{
        capabilities::build_host_capabilities,
        cli::{CliPromptAdapter, JsonPromptAdapter},
        config_patch::{
            apply_config_patch, default_config_patch_path, restore_config, snapshot_config,
        },
        flow_runner::run_bootstrap_flow,
        http_adapter::HttpPromptAdapter,
        network::{NetAllowList, NetworkPolicy},
        output::BootstrapOutput,
        secrets::{
            SecretsBackend, execute_writes, parse_backend, restore_backend, snapshot_backend,
        },
        state::{BootstrapState, ensure_upgrade_allowed, load_state_backend, save_state_backend},
    },
    config::{
        BootstrapStateBackend, CliArgs, Command, DeployerConfig, InteractionMode,
        PlatformActionArgs, PlatformArgs, PlatformCommand,
    },
    platform::oci::resolve_oci_pack,
    platform::{self, VerificationPolicy},
};

#[tokio::main]
async fn main() {
    let cli = CliArgs::parse();
    match cli.command {
        Command::Platform(args) => handle_platform(args),
        _ => handle_standard(cli).await,
    }
}

fn handle_platform(args: PlatformArgs) {
    let backend = match parse_backend(&args.secrets_backend) {
        Ok(backend) => backend,
        Err(err) => {
            eprintln!("invalid secrets backend: {err}");
            std::process::exit(1);
        }
    };

    let context = PlatformContext::from_args(&args, backend);

    match args.command {
        PlatformCommand::Install(cmd) => summarize_pack("install", &cmd, &context),
        PlatformCommand::Upgrade(cmd) => summarize_pack("upgrade", &cmd, &context),
        PlatformCommand::Status => {
            match load_state_backend(
                context.state_backend,
                &context.state_path,
                &context.k8s_namespace,
                &context.k8s_state_name,
            ) {
                Ok(Some(state)) => {
                    println!(
                        "bootstrap state:\n- version: {}\n- digest: {}\n- installed_at: {:?}\n- environment_kind: {}\n- last_upgrade_at: {:?}\n- rollback_ref: {}",
                        state.version.as_deref().unwrap_or("unknown"),
                        state.digest.as_deref().unwrap_or("unknown"),
                        state.installed_at,
                        state.environment_kind.as_deref().unwrap_or("unknown"),
                        state.last_upgrade_at,
                        state.rollback_ref.as_deref().unwrap_or("none"),
                    );
                }
                Ok(None) => println!("bootstrap state: not installed (no state file found)"),
                Err(err) => {
                    eprintln!(
                        "failed to read bootstrap state at {}: {err}",
                        args.bootstrap_state.display()
                    );
                    std::process::exit(1);
                }
            }
            eprintln!(
                "\nplatform status reporting is read-only; install/upgrade execution is not implemented yet."
            );
            std::process::exit(0);
        }
    }
}

#[derive(Clone)]
struct PlatformContext {
    interaction: InteractionMode,
    allow_listeners: bool,
    allow_network: bool,
    net_allowlist: Option<String>,
    offline_only: bool,
    bind: Option<String>,
    interaction_timeout: Duration,
    secrets_backend: SecretsBackend,
    answers: Option<String>,
    output_path: Option<PathBuf>,
    config_out: Option<PathBuf>,
    state_path: PathBuf,
    state_backend: BootstrapStateBackend,
    k8s_namespace: String,
    k8s_state_name: String,
}

impl PlatformContext {
    fn from_args(args: &PlatformArgs, secrets_backend: SecretsBackend) -> Self {
        Self {
            interaction: args.interaction,
            allow_listeners: args.allow_listeners,
            allow_network: args.allow_network,
            net_allowlist: args.net_allowlist.clone(),
            offline_only: args.offline_only,
            bind: args.bind.clone(),
            interaction_timeout: Duration::from_secs(args.interaction_timeout),
            secrets_backend,
            answers: args.answers.clone(),
            output_path: args.output.clone(),
            config_out: args.config_out.clone(),
            state_path: args.bootstrap_state.clone(),
            state_backend: args.bootstrap_state_backend,
            k8s_namespace: args.k8s_namespace.clone(),
            k8s_state_name: args.k8s_state_name.clone(),
        }
    }
}

async fn handle_standard(cli: CliArgs) {
    match DeployerConfig::from_env_and_args(cli) {
        Ok(config) => {
            if !config.config_warnings.is_empty() {
                eprintln!("configuration warnings:");
                for warning in &config.config_warnings {
                    eprintln!("- {warning}");
                }
            }
            if config.explain_config || config.explain_config_json {
                if config.explain_config_json {
                    let payload = serde_json::json!({
                        "config": &config.greentic,
                        "provenance": &config.provenance,
                        "warnings": &config.config_warnings,
                    });
                    println!("{}", serde_json::to_string_pretty(&payload).unwrap());
                } else {
                    println!("greentic config (resolved):");
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&config.greentic).unwrap()
                    );
                    println!("provenance: {:#?}", config.provenance);
                    if !config.config_warnings.is_empty() {
                        println!("warnings:");
                        for w in &config.config_warnings {
                            println!("- {w}");
                        }
                    }
                }
                return;
            }
            if let Err(err) = apply::run(config).await {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        }
        Err(err) => {
            eprintln!("configuration error: {err}");
            std::process::exit(1);
        }
    }
}

fn summarize_pack(action: &str, cmd: &PlatformActionArgs, ctx: &PlatformContext) {
    let net_allowlist = match NetAllowList::parse(ctx.net_allowlist.as_deref()) {
        Ok(list) => list,
        Err(err) => {
            eprintln!("invalid network allowlist: {err}");
            std::process::exit(1);
        }
    };
    let network_policy = NetworkPolicy::new(ctx.allow_network, ctx.offline_only, net_allowlist);
    let cache_base = ctx
        .state_path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let pack_path = if cmd.pack.starts_with("oci://") {
        match resolve_oci_pack(&cmd.pack, &cache_base, &network_policy) {
            Ok(path) => path,
            Err(err) => {
                eprintln!("failed to resolve oci pack {}: {err}", cmd.pack);
                std::process::exit(1);
            }
        }
    } else {
        PathBuf::from(&cmd.pack)
    };
    match platform::load_platform_pack(&pack_path) {
        Ok(info) => {
            let policy = VerificationPolicy {
                verify: cmd.verify,
                strict: cmd.strict_verify,
            };
            if let Err(err) = platform::verify_platform_pack(&info, policy) {
                eprintln!("platform {action} verification failed: {err}");
                std::process::exit(1);
            }
            let current_state = match load_state_backend(
                ctx.state_backend,
                &ctx.state_path,
                &ctx.k8s_namespace,
                &ctx.k8s_state_name,
            ) {
                Ok(state) => state,
                Err(err) => {
                    eprintln!(
                        "failed to read bootstrap state at {}: {err}",
                        ctx.state_path.display()
                    );
                    std::process::exit(1);
                }
            };
            if action == "upgrade"
                && let Err(err) =
                    ensure_upgrade_allowed(current_state.clone(), &info.manifest.version)
            {
                eprintln!("upgrade preflight failed: {err}");
                std::process::exit(1);
            }
            let digest = info.digest.clone().unwrap_or_else(|| "not computed".into());
            let summary = format!(
                "platform {} preview:\n- pack: {}\n- version: {}\n- digest: {}",
                action, info.manifest.pack_id, info.manifest.version, digest
            );
            println!("{summary}");
            let flow_bytes = match platform::load_bootstrap_flow(
                &pack_path,
                &info.manifest,
                action == "install",
            ) {
                Ok(bytes) => bytes,
                Err(err) => {
                    eprintln!("failed to load bootstrap flow: {err}");
                    std::process::exit(1);
                }
            };
            let capabilities =
                build_host_capabilities(ctx.interaction, ctx.allow_listeners, &network_policy);
            println!("host capabilities: {:?}", capabilities);
            if !capabilities.disabled_reasons.is_empty() {
                for reason in &capabilities.disabled_reasons {
                    println!("adapter disabled: {reason}");
                }
            }
            let result = match ctx.interaction {
                InteractionMode::Cli | InteractionMode::Auto => {
                    let stdin = std::io::stdin();
                    let stdout = std::io::stdout();
                    let mut adapter = CliPromptAdapter::new(stdin.lock(), stdout);
                    run_bootstrap_flow(&flow_bytes, &mut adapter)
                }
                InteractionMode::Http => {
                    if network_policy.offline_only() {
                        eprintln!("http interaction not allowed in offline-only mode");
                        std::process::exit(1);
                    }
                    if !ctx.allow_listeners || !network_policy.allow_network() {
                        eprintln!(
                            "http interaction requires --allow-listeners and --allow-network"
                        );
                        std::process::exit(1);
                    }
                    let bind_addr = ctx
                        .bind
                        .clone()
                        .unwrap_or_else(|| "127.0.0.1:0".to_string());
                    let mut adapter = HttpPromptAdapter::bind(&bind_addr, ctx.interaction_timeout)
                        .unwrap_or_else(|err| {
                            eprintln!("failed to bind http adapter at {bind_addr}: {err}");
                            std::process::exit(1);
                        });
                    println!("http adapter listening at http://{}", adapter.bound_addr());
                    run_bootstrap_flow(&flow_bytes, &mut adapter)
                }
                InteractionMode::Json => {
                    let answers = match ctx.answers {
                        Some(ref path) => load_answers(path),
                        None => {
                            eprintln!("--answers is required when --interaction json");
                            std::process::exit(1);
                        }
                    };
                    let mut adapter = JsonPromptAdapter::new(answers).unwrap_or_else(|err| {
                        eprintln!("invalid answers payload: {err}");
                        std::process::exit(1);
                    });
                    run_bootstrap_flow(&flow_bytes, &mut adapter)
                }
                InteractionMode::Mqtt => {
                    eprintln!("mqtt interaction not yet wired into deployer runtime");
                    std::process::exit(1);
                }
            };
            match result {
                Ok(result) => {
                    let config_path = ctx
                        .config_out
                        .clone()
                        .unwrap_or_else(|| default_config_patch_path(&ctx.state_path));
                    let config_snapshot = snapshot_config(&config_path).unwrap_or_else(|err| {
                        eprintln!(
                            "failed to snapshot config at {}: {err}",
                            config_path.display()
                        );
                        std::process::exit(1);
                    });
                    let secrets_snapshot = snapshot_backend(&ctx.secrets_backend);
                    if let Err(err) =
                        execute_writes(&ctx.secrets_backend, &result.output.secrets_writes)
                    {
                        if let Some(snapshot) = secrets_snapshot {
                            let _ = restore_backend(&snapshot);
                        }
                        eprintln!("secret write failed: {err}");
                        std::process::exit(1);
                    }
                    if let Err(err) = apply_config_patch(&config_path, &result.output.config_patch)
                    {
                        if let Some(snapshot) = secrets_snapshot {
                            let _ = restore_backend(&snapshot);
                        }
                        let _ = restore_config(&config_path, &config_snapshot);
                        eprintln!(
                            "failed to apply config patch to {}: {err}",
                            config_path.display()
                        );
                        std::process::exit(1);
                    }
                    if let Err(err) = run_install_plan(&pack_path) {
                        if let Some(snapshot) = secrets_snapshot {
                            let _ = restore_backend(&snapshot);
                        }
                        let _ = restore_config(&config_path, &config_snapshot);
                        eprintln!("deploy plan failed: {err}");
                        std::process::exit(1);
                    }
                    let state = if action == "upgrade" {
                        let existing = load_state_backend(
                            ctx.state_backend,
                            &ctx.state_path,
                            &ctx.k8s_namespace,
                            &ctx.k8s_state_name,
                        )
                        .unwrap_or(None)
                        .expect("upgrade preflight ensures state exists");
                        let rollback_ref = format!(
                            "version={},digest={}",
                            existing.version.as_deref().unwrap_or("unknown"),
                            existing.digest.as_deref().unwrap_or("unknown")
                        );
                        BootstrapState::upgraded_from(
                            &existing,
                            Some(info.manifest.version.to_string()),
                            info.digest.clone(),
                            Some(rollback_ref),
                        )
                    } else {
                        BootstrapState::installed_now(
                            Some(info.manifest.version.to_string()),
                            info.digest.clone(),
                        )
                    };
                    if let Err(err) = save_state_backend(
                        ctx.state_backend,
                        &ctx.state_path,
                        &ctx.k8s_namespace,
                        &ctx.k8s_state_name,
                        &state,
                    ) {
                        if let Some(snapshot) = secrets_snapshot {
                            let _ = restore_backend(&snapshot);
                        }
                        let _ = restore_config(&config_path, &config_snapshot);
                        eprintln!(
                            "failed to persist bootstrap state at {}: {err}",
                            ctx.state_path.display()
                        );
                        std::process::exit(1);
                    }
                    render_bootstrap_output(&result.output);
                    if let Some(path) = ctx.output_path.as_ref()
                        && let Err(err) = write_output_file(path, &result.output)
                    {
                        eprintln!("failed to write output file: {err}");
                        std::process::exit(1);
                    }
                }
                Err(err) => {
                    eprintln!("bootstrap flow failed: {err}");
                    std::process::exit(1);
                }
            }
            std::process::exit(0);
        }
        Err(err) => {
            eprintln!(
                "platform {action} failed to load pack {}: {err}",
                pack_path.display()
            );
            std::process::exit(1);
        }
    }
}

fn render_bootstrap_output(output: &BootstrapOutput) {
    let redacted = output.redacted();
    match serde_json::to_string_pretty(&redacted) {
        Ok(json) => println!("bootstrap output (json):\n{json}"),
        Err(err) => println!("bootstrap output serialization failed: {err}"),
    }
}

fn write_output_file(path: &std::path::Path, output: &BootstrapOutput) -> Result<(), String> {
    let redacted = output.redacted();
    let serialized = serde_json::to_string_pretty(&redacted).map_err(|err| err.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    std::fs::write(path, serialized).map_err(|err| err.to_string())
}

fn load_answers(source: &str) -> serde_json::Value {
    if source == "@-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .expect("read stdin for answers");
        serde_json::from_str(&buf).unwrap_or_else(|err| {
            eprintln!("failed to parse answers from stdin: {err}");
            std::process::exit(1);
        })
    } else {
        let content = std::fs::read_to_string(source).unwrap_or_else(|err| {
            eprintln!("failed to read answers file {source}: {err}");
            std::process::exit(1);
        });
        serde_json::from_str(&content).unwrap_or_else(|err| {
            eprintln!("failed to parse answers file {source}: {err}");
            std::process::exit(1);
        })
    }
}

/// Placeholder hook to reuse deployment machinery (will wire real apply later).
fn run_install_plan(_pack_path: &std::path::Path) -> Result<(), String> {
    // The platform pack has already been validated and parsed; real deploy logic will
    // reuse existing plan/apply machinery in a future PR.
    Ok(())
}
