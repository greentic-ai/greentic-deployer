use std::collections::HashMap;
use std::env;
use std::sync::{Arc, RwLock};

use crate::config::DeployerConfig;
use crate::error::{DeployerError, Result};
use crate::plan::PlanContext;
use async_trait::async_trait;
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
    let prefix = format!(
        "DEPLOY_TARGET_{}_{}",
        sanitize_key(&target.provider),
        sanitize_key(&target.strategy)
    );
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
    use std::env;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::tempdir_in;

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
            pack_ref: None,
            distributor_url: None,
            distributor_token: None,
            yes: true,
            preview: false,
            dry_run: false,
            iac_tool: IaCTool::Terraform,
            output: crate::config::OutputFormat::Text,
            greentic: greentic_config_types::GreenticConfig::default(),
            provenance: greentic_config::ProvenanceMap::new(),
            config_warnings: Vec::new(),
            explain_config: false,
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
        let dir = tempdir_in(env::current_dir().expect("cwd")).expect("temp dir");
        let manifest = PackManifest {
            schema_version: "pack-v1".to_string(),
            pack_id: PackId::try_from("dev.greentic.sample").unwrap(),
            version: Version::new(0, 1, 0),
            kind: PackKind::Application,
            publisher: "greentic".to_string(),
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
        };
        let bytes = encode_pack_manifest(&manifest).expect("encode manifest");
        std::fs::write(dir.path().join("manifest.cbor"), bytes).expect("write manifest");
        dir.into_path()
    }
}
