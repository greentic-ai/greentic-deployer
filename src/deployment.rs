use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::config::DeployerConfig;
use crate::error::{DeployerError, Result};
use crate::pack_introspect::{read_manifest_from_directory, read_manifest_from_gtpack};
use crate::plan::PlanContext;
use async_trait::async_trait;
use greentic_types::pack_manifest::PackManifest;
use once_cell::sync::Lazy;

/// Logical deployment target keyed by provider + strategy.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeploymentTarget {
    pub provider: String,
    pub strategy: String,
}

/// Dispatch details describing which deployment pack/flow to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentDispatch {
    pub pack_id: String,
    pub flow_id: String,
}

/// Resolved deployment pack selection including discovered manifest.
#[derive(Debug)]
pub struct DeploymentPackSelection {
    pub dispatch: DeploymentDispatch,
    pub pack_path: PathBuf,
    pub manifest: PackManifest,
    pub origin: String,
    pub candidates: Vec<String>,
}

/// Built-in placeholder defaults. Real packs should replace these entries later.
pub fn default_dispatch_table() -> HashMap<DeploymentTarget, DeploymentDispatch> {
    let mut map = HashMap::new();
    map.insert(
        DeploymentTarget {
            provider: "aws".into(),
            strategy: "iac-only".into(),
        },
        DeploymentDispatch {
            pack_id: "greentic.demo.deploy.aws".into(),
            flow_id: "deploy_aws_iac".into(),
        },
    );
    map.insert(
        DeploymentTarget {
            provider: "local".into(),
            strategy: "iac-only".into(),
        },
        DeploymentDispatch {
            pack_id: "greentic.demo.deploy.local".into(),
            flow_id: "deploy_local_iac".into(),
        },
    );
    map.insert(
        DeploymentTarget {
            provider: "azure".into(),
            strategy: "iac-only".into(),
        },
        DeploymentDispatch {
            pack_id: "greentic.demo.deploy.azure".into(),
            flow_id: "deploy_azure_iac".into(),
        },
    );
    map.insert(
        DeploymentTarget {
            provider: "gcp".into(),
            strategy: "iac-only".into(),
        },
        DeploymentDispatch {
            pack_id: "greentic.demo.deploy.gcp".into(),
            flow_id: "deploy_gcp_iac".into(),
        },
    );
    map.insert(
        DeploymentTarget {
            provider: "k8s".into(),
            strategy: "iac-only".into(),
        },
        DeploymentDispatch {
            pack_id: "greentic.demo.deploy.k8s".into(),
            flow_id: "deploy_k8s_iac".into(),
        },
    );
    map.insert(
        DeploymentTarget {
            provider: "generic".into(),
            strategy: "iac-only".into(),
        },
        DeploymentDispatch {
            pack_id: "greentic.demo.deploy.generic".into(),
            flow_id: "deploy_generic_iac".into(),
        },
    );
    map
}

/// Resolve the dispatch entry for a target, honoring environment overrides.
pub fn resolve_dispatch(target: &DeploymentTarget) -> Result<DeploymentDispatch> {
    resolve_dispatch_with_env(target, |key| env::var(key).ok())
}

pub fn resolve_deployment_pack(
    config: &DeployerConfig,
    target: &DeploymentTarget,
) -> Result<DeploymentPackSelection> {
    let dispatch = resolve_dispatch(target)?;
    let discovery = find_pack_for_dispatch(config, target, &dispatch)?;
    ensure_flow_available(&dispatch, &discovery.manifest)?;
    Ok(DeploymentPackSelection {
        dispatch,
        pack_path: discovery.pack_path,
        manifest: discovery.manifest,
        origin: discovery.origin,
        candidates: discovery.candidates,
    })
}

fn resolve_dispatch_with_env<F>(target: &DeploymentTarget, get_env: F) -> Result<DeploymentDispatch>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(dispatch) = env_override(target, &get_env)? {
        return Ok(dispatch);
    }

    let mut defaults = default_dispatch_table();
    if let Some(dispatch) = defaults.remove(target) {
        return Ok(dispatch);
    }

    Err(DeployerError::Config(format!(
        "No deployment pack mapping for provider={} strategy={}. Configure DEPLOY_TARGET_{}_{}_PACK_ID / _FLOW_ID or extend the defaults.",
        target.provider,
        target.strategy,
        sanitize_key(&target.provider),
        sanitize_key(&target.strategy),
    )))
}

fn env_override<F>(target: &DeploymentTarget, get_env: &F) -> Result<Option<DeploymentDispatch>>
where
    F: Fn(&str) -> Option<String>,
{
    let strategy_prefix = format!(
        "DEPLOY_TARGET_{}_{}",
        sanitize_key(&target.provider),
        sanitize_key(&target.strategy)
    );
    if let Some(dispatch) = env_override_with_prefix(&strategy_prefix, get_env)? {
        return Ok(Some(dispatch));
    }
    let provider_prefix = format!("DEPLOY_TARGET_{}", sanitize_key(&target.provider));
    env_override_with_prefix(&provider_prefix, get_env)
}

fn env_override_with_prefix<F>(prefix: &str, get_env: &F) -> Result<Option<DeploymentDispatch>>
where
    F: Fn(&str) -> Option<String>,
{
    let pack_key = format!("{prefix}_PACK_ID");
    let flow_key = format!("{prefix}_FLOW_ID");
    let pack = get_env(&pack_key);
    let flow = get_env(&flow_key);
    match (pack, flow) {
        (Some(pack_id), Some(flow_id)) => Ok(Some(DeploymentDispatch { pack_id, flow_id })),
        (None, None) => Ok(None),
        (Some(_), None) | (None, Some(_)) => Err(DeployerError::Config(format!(
            "Incomplete deployment mapping overrides. Both {pack_key} and {flow_key} must be set."
        ))),
    }
}

struct SearchPath {
    label: &'static str,
    path: PathBuf,
}

struct PackDiscovery {
    pack_path: PathBuf,
    manifest: PackManifest,
    origin: String,
    candidates: Vec<String>,
}

fn find_pack_for_dispatch(
    config: &DeployerConfig,
    target: &DeploymentTarget,
    dispatch: &DeploymentDispatch,
) -> Result<PackDiscovery> {
    if let Some(ref override_path) = config.provider_pack {
        let manifest = load_manifest(override_path)?;
        let actual = manifest.pack_id.to_string();
        if actual != dispatch.pack_id {
            return Err(DeployerError::Config(format!(
                "explicit deployment pack {} contains pack_id {} (expected {})",
                override_path.display(),
                actual,
                dispatch.pack_id
            )));
        }
        return Ok(PackDiscovery {
            pack_path: override_path.clone(),
            manifest,
            origin: format!("override -> {}", override_path.display()),
            candidates: vec![format!("{} (override {})", actual, override_path.display())],
        });
    }

    if let Some((direct_path, manifest)) =
        resolve_direct_pack_path(config, target).and_then(|direct_path| {
            if !direct_path.exists() {
                return None;
            }
            match load_manifest(&direct_path) {
                Ok(manifest) if manifest.pack_id.to_string() == dispatch.pack_id => {
                    Some((direct_path, manifest))
                }
                _ => None,
            }
        })
    {
        let candidate_display = direct_path.display().to_string();
        let entry = format!("{} ({})", manifest.pack_id, candidate_display);
        return Ok(PackDiscovery {
            pack_path: direct_path.clone(),
            manifest,
            origin: format!("providers-dir -> {}", candidate_display),
            candidates: vec![entry],
        });
    }

    let search_paths = build_search_paths(config);
    let mut candidates = Vec::new();
    for search in &search_paths {
        for candidate in gather_candidates(&search.path) {
            if let Ok(manifest) = load_manifest(&candidate) {
                let entry = format!("{} ({})", manifest.pack_id, candidate.display());
                candidates.push(entry.clone());
                if manifest.pack_id.to_string() == dispatch.pack_id {
                    let candidate_display = candidate.display().to_string();
                    let pack_path = candidate.clone();
                    return Ok(PackDiscovery {
                        pack_path,
                        manifest,
                        origin: format!("{} -> {}", search.label, candidate_display),
                        candidates,
                    });
                }
            }
        }
    }

    let summary = build_search_summary(&search_paths);
    Err(DeployerError::Config(format!(
        "Deployment pack {} not found; searched {} (candidates: {})",
        dispatch.pack_id,
        summary,
        if candidates.is_empty() {
            "none".into()
        } else {
            candidates.join("; ")
        }
    )))
}

fn ensure_flow_available(dispatch: &DeploymentDispatch, manifest: &PackManifest) -> Result<()> {
    let available: Vec<String> = manifest
        .flows
        .iter()
        .map(|entry| entry.id.to_string())
        .collect();
    if available.iter().any(|flow| flow == &dispatch.flow_id) {
        return Ok(());
    }

    Err(DeployerError::Config(format!(
        "Flow {} not found in {} (available flows: {})",
        dispatch.flow_id,
        dispatch.pack_id,
        if available.is_empty() {
            "none".into()
        } else {
            available.join(", ")
        }
    )))
}

fn build_search_paths(config: &DeployerConfig) -> Vec<SearchPath> {
    vec![
        SearchPath {
            label: "providers-dir",
            path: config.providers_dir.clone(),
        },
        SearchPath {
            label: "packs-dir",
            path: config.packs_dir.clone(),
        },
        SearchPath {
            label: "dist",
            path: PathBuf::from("dist"),
        },
        SearchPath {
            label: "examples",
            path: PathBuf::from("examples"),
        },
    ]
}

fn resolve_direct_pack_path(config: &DeployerConfig, target: &DeploymentTarget) -> Option<PathBuf> {
    let pack_path = config.providers_dir.join(&target.provider);
    if pack_path.exists() {
        Some(pack_path)
    } else {
        None
    }
}

fn gather_candidates(path: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let candidate = entry.path();
            if candidate.is_dir()
                || candidate.extension().and_then(|ext| ext.to_str()) == Some("gtpack")
            {
                candidates.push(candidate);
            }
        }
    }
    candidates
}

fn load_manifest(path: &Path) -> Result<PackManifest> {
    if path.is_dir() {
        read_manifest_from_directory(path)
    } else {
        read_manifest_from_gtpack(path)
    }
}

fn build_search_summary(paths: &[SearchPath]) -> String {
    paths
        .iter()
        .map(|entry| format!("{} ({})", entry.label, entry.path.display()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn sanitize_key(input: &str) -> String {
    input
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// Placeholder hook for future deployment-pack execution via greentic-runner.
///
/// Returns `Ok(true)` when the plan was executed via a deployment pack,
/// `Ok(false)` when the legacy provider shim should be used, and `Err` on fatal failures.
pub async fn execute_deployment_pack(
    config: &DeployerConfig,
    plan: &PlanContext,
    dispatch: &DeploymentDispatch,
) -> Result<bool> {
    if let Some(executor) = deployment_executor() {
        executor.execute(config, plan, dispatch).await?;
        return Ok(true);
    }
    tracing::info!(
        provider = %plan.deployment.provider,
        strategy = %plan.deployment.strategy,
        pack_id = %dispatch.pack_id,
        flow_id = %dispatch.flow_id,
        "deployment executor not registered; falling back to legacy shim"
    );
    Ok(false)
}

#[async_trait]
pub trait DeploymentExecutor: Send + Sync {
    async fn execute(
        &self,
        config: &DeployerConfig,
        plan: &PlanContext,
        dispatch: &DeploymentDispatch,
    ) -> Result<()>;
}

static EXECUTOR: Lazy<RwLock<Option<Arc<dyn DeploymentExecutor>>>> =
    Lazy::new(|| RwLock::new(None));

pub fn set_deployment_executor(executor: Arc<dyn DeploymentExecutor>) {
    let mut slot = EXECUTOR.write().expect("deployment executor lock poisoned");
    *slot = Some(executor);
}

#[cfg(test)]
pub fn clear_deployment_executor() {
    let mut slot = EXECUTOR.write().expect("deployment executor lock poisoned");
    *slot = None;
}

fn deployment_executor() -> Option<Arc<dyn DeploymentExecutor>> {
    EXECUTOR
        .read()
        .expect("deployment executor lock poisoned")
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Action, DeployerConfig, Provider};
    use crate::iac::IaCTool;
    use crate::pack_introspect;
    use greentic_types::cbor::encode_pack_manifest;
    use greentic_types::component::{ComponentCapabilities, ComponentManifest, ComponentProfiles};
    use greentic_types::pack_manifest::{PackKind, PackManifest};
    use greentic_types::{ComponentId, PackId};
    use semver::Version;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn resolves_default_entry() {
        let target = DeploymentTarget {
            provider: "generic".into(),
            strategy: "iac-only".into(),
        };
        let dispatch = resolve_dispatch(&target).expect("default mapping");
        assert_eq!(dispatch.pack_id, "greentic.demo.deploy.generic");
        assert_eq!(dispatch.flow_id, "deploy_generic_iac");
    }

    #[test]
    fn honors_env_override() {
        let target = DeploymentTarget {
            provider: "aws".into(),
            strategy: "serverless".into(),
        };
        let dispatch = resolve_dispatch_with_env(&target, |key| match key {
            "DEPLOY_TARGET_AWS_SERVERLESS_PACK_ID" => Some("custom.pack".into()),
            "DEPLOY_TARGET_AWS_SERVERLESS_FLOW_ID" => Some("flow_one".into()),
            _ => None,
        })
        .expect("env mapping");
        assert_eq!(dispatch.pack_id, "custom.pack");
        assert_eq!(dispatch.flow_id, "flow_one");
    }

    #[test]
    fn honors_provider_only_override() {
        let target = DeploymentTarget {
            provider: "aws".into(),
            strategy: "serverless".into(),
        };
        let dispatch = resolve_dispatch_with_env(&target, |key| match key {
            "DEPLOY_TARGET_AWS_PACK_ID" => Some("provider.pack".into()),
            "DEPLOY_TARGET_AWS_FLOW_ID" => Some("provider_flow".into()),
            _ => None,
        })
        .expect("provider fallback");
        assert_eq!(dispatch.pack_id, "provider.pack");
        assert_eq!(dispatch.flow_id, "provider_flow");
    }

    #[test]
    fn errors_when_override_incomplete() {
        let target = DeploymentTarget {
            provider: "aws".into(),
            strategy: "serverless".into(),
        };
        let err = resolve_dispatch_with_env(&target, |key| {
            if key == "DEPLOY_TARGET_AWS_SERVERLESS_PACK_ID" {
                Some("only-pack".into())
            } else {
                None
            }
        })
        .expect_err("missing flow");
        assert!(format!("{err}").contains("Incomplete deployment mapping overrides"));
    }

    struct TestExecutor {
        hits: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl DeploymentExecutor for TestExecutor {
        async fn execute(
            &self,
            _config: &DeployerConfig,
            _plan: &PlanContext,
            _dispatch: &DeploymentDispatch,
        ) -> Result<()> {
            self.hits.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn executes_via_registered_executor() {
        clear_deployment_executor();
        let hits = Arc::new(AtomicUsize::new(0));
        set_deployment_executor(Arc::new(TestExecutor { hits: hits.clone() }));
        let pack_path = write_test_pack();
        let config = DeployerConfig {
            action: Action::Plan,
            provider: Provider::Aws,
            strategy: "iac-only".into(),
            tenant: "acme".into(),
            environment: "staging".into(),
            pack_path,
            providers_dir: PathBuf::from("providers/deployer"),
            packs_dir: PathBuf::from("packs"),
            provider_pack: None,
            pack_ref: None,
            distributor_url: None,
            distributor_token: None,
            yes: true,
            preview: false,
            dry_run: false,
            iac_tool: IaCTool::Terraform,
            output: crate::config::OutputFormat::Text,
            greentic: greentic_config::ConfigResolver::new()
                .load()
                .expect("load default config")
                .config,
            provenance: greentic_config::ProvenanceMap::new(),
            config_warnings: Vec::new(),
            explain_config: false,
            explain_config_json: false,
            allow_remote_in_offline: false,
        };
        let plan = pack_introspect::build_plan(&config).expect("plan builds");
        let dispatch = DeploymentDispatch {
            pack_id: "test.pack".into(),
            flow_id: "deploy_flow".into(),
        };
        let ran = execute_deployment_pack(&config, &plan, &dispatch)
            .await
            .expect("executor runs");
        assert!(ran);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        clear_deployment_executor();
    }

    #[allow(deprecated)]
    fn write_test_pack() -> PathBuf {
        let base = env::current_dir().expect("cwd").join("target/tmp-tests");
        std::fs::create_dir_all(&base).expect("create tmp base");
        let dir = tempfile::tempdir_in(base).expect("temp dir");
        let manifest = PackManifest {
            schema_version: "pack-v1".to_string(),
            pack_id: PackId::try_from("dev.greentic.sample").unwrap(),
            version: Version::new(0, 1, 0),
            kind: PackKind::Application,
            publisher: "greentic".to_string(),
            secret_requirements: Vec::new(),
            components: vec![ComponentManifest {
                id: ComponentId::try_from("dev.greentic.component").unwrap(),
                version: Version::new(0, 1, 0),
                supports: Vec::new(),
                world: "greentic:test/world".to_string(),
                profiles: ComponentProfiles::default(),
                capabilities: ComponentCapabilities::default(),
                configurators: None,
                operations: Vec::new(),
                config_schema: None,
                resources: Default::default(),
                dev_flows: Default::default(),
            }],
            flows: Vec::new(),
            dependencies: Vec::new(),
            capabilities: Vec::new(),
            signatures: Default::default(),
            bootstrap: None,
            extensions: None,
        };
        let bytes = encode_pack_manifest(&manifest).expect("encode manifest");
        std::fs::write(dir.path().join("manifest.cbor"), bytes).expect("write manifest");
        dir.into_path()
    }
}
