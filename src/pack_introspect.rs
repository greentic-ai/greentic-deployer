use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use greentic_pack::builder::PackManifest;
use greentic_pack::plan::infer_base_deployment_plan;
use greentic_types::component::ComponentManifest;
use greentic_types::deployment::{DeploymentPlan, OAuthPlan, SecretPlan};
use greentic_types::{EnvId, TenantCtx, TenantId};
use serde_cbor;
use serde_json::{self, Map as JsonMap, Value as JsonValue};
use zip::ZipArchive;

use crate::config::DeployerConfig;
use crate::error::{DeployerError, Result};
use crate::plan::{DeploymentHints, PlanContext, assemble_plan};

/// Build a plan context from the provided pack.
pub fn build_plan(config: &DeployerConfig) -> Result<PlanContext> {
    let mut source = PackSource::open(&config.pack_path)?;
    let manifest = source.read_manifest()?;
    let components = source.load_component_manifests(&manifest)?;
    let tenant_ctx = build_tenant_ctx(config)?;
    let connectors = manifest.meta.annotations.get("connectors");
    let mut base = infer_base_deployment_plan(
        &manifest.meta,
        &manifest.flows,
        connectors,
        &components,
        &tenant_ctx,
        &config.environment,
    );
    merge_annotation_secrets(&mut base, &manifest.meta.annotations);
    merge_annotation_oauth(&mut base, &manifest.meta.annotations, config);
    let deployment = build_deployment_hints(&manifest.meta.annotations, config);
    Ok(assemble_plan(base, config, deployment))
}

struct PackSource {
    inner: PackSourceInner,
}

enum PackSourceInner {
    Archive(ZipArchive<File>),
    Directory(PathBuf),
}

impl PackSource {
    fn open(path: &Path) -> Result<Self> {
        if path.is_dir() {
            Ok(Self {
                inner: PackSourceInner::Directory(path.to_path_buf()),
            })
        } else {
            let file = File::open(path)?;
            let archive = ZipArchive::new(file)?;
            Ok(Self {
                inner: PackSourceInner::Archive(archive),
            })
        }
    }

    fn read_manifest(&mut self) -> Result<PackManifest> {
        match &mut self.inner {
            PackSourceInner::Archive(archive) => {
                let mut manifest = Vec::new();
                let mut entry = archive.by_name("manifest.cbor")?;
                entry.read_to_end(&mut manifest)?;
                Ok(serde_cbor::from_slice(&manifest)?)
            }
            PackSourceInner::Directory(dir) => read_manifest_from_directory(dir),
        }
    }

    fn load_component_manifests(
        &mut self,
        manifest: &PackManifest,
    ) -> Result<HashMap<String, ComponentManifest>> {
        let mut components = HashMap::new();
        for entry in &manifest.components {
            if let Some(path) = &entry.manifest_file {
                let json = self.read_file_to_string(path)?;
                let parsed: ComponentManifest = serde_json::from_str(&json).map_err(|err| {
                    DeployerError::Pack(format!("component manifest {} is invalid: {}", path, err))
                })?;
                components.insert(parsed.id.to_string(), parsed);
            }
        }
        Ok(components)
    }

    fn read_file_to_string(&mut self, relative_path: &str) -> Result<String> {
        match &mut self.inner {
            PackSourceInner::Archive(archive) => {
                let mut entry = archive.by_name(relative_path)?;
                let mut contents = String::new();
                entry.read_to_string(&mut contents)?;
                Ok(contents)
            }
            PackSourceInner::Directory(root) => {
                let path = root.join(relative_path);
                let contents = fs::read_to_string(path)?;
                Ok(contents)
            }
        }
    }
}

fn read_manifest_from_directory(root: &Path) -> Result<PackManifest> {
    let cbor = root.join("manifest.cbor");
    let json = root.join("manifest.json");

    if cbor.exists() {
        let bytes = fs::read(cbor)?;
        Ok(serde_cbor::from_slice(&bytes)?)
    } else if json.exists() {
        let bytes = fs::read(json)?;
        Ok(serde_json::from_slice(&bytes)?)
    } else {
        Err(DeployerError::Pack(format!(
            "pack manifest missing in {}",
            root.display()
        )))
    }
}

fn build_tenant_ctx(config: &DeployerConfig) -> Result<TenantCtx> {
    let env_id = EnvId::from_str(&config.environment).map_err(|err| {
        DeployerError::Config(format!(
            "invalid environment '{}': {}",
            config.environment, err
        ))
    })?;
    let tenant_id = TenantId::from_str(&config.tenant).map_err(|err| {
        DeployerError::Config(format!("invalid tenant '{}': {}", config.tenant, err))
    })?;
    Ok(TenantCtx::new(env_id, tenant_id))
}

fn merge_annotation_secrets(plan: &mut DeploymentPlan, annotations: &JsonMap<String, JsonValue>) {
    let Some(secret_map) = annotations
        .get("greentic.secrets")
        .and_then(|value| value.as_object())
    else {
        return;
    };

    for key in secret_map.keys() {
        if plan.secrets.iter().any(|entry| entry.key == *key) {
            continue;
        }
        plan.secrets.push(SecretPlan {
            key: key.clone(),
            required: true,
            scope: "tenant".to_string(),
        });
    }
}

fn merge_annotation_oauth(
    plan: &mut DeploymentPlan,
    annotations: &JsonMap<String, JsonValue>,
    config: &DeployerConfig,
) {
    let Some(oauth_map) = annotations
        .get("greentic.oauth")
        .and_then(|value| value.as_object())
    else {
        return;
    };

    if oauth_map.is_empty() {
        return;
    }

    plan.oauth = oauth_map
        .iter()
        .map(|(provider, entry)| {
            let logical_client_id = entry
                .get("client_id")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string())
                .unwrap_or_else(|| format!("{}-{}-{provider}", config.tenant, config.environment));

            let redirect_path = format!(
                "/oauth/{provider}/callback/{tenant}/{environment}",
                tenant = config.tenant,
                environment = config.environment
            );

            OAuthPlan {
                provider_id: provider.clone(),
                logical_client_id,
                redirect_path,
                extra: entry.clone(),
            }
        })
        .collect();
}

fn build_deployment_hints(
    annotations: &JsonMap<String, JsonValue>,
    config: &DeployerConfig,
) -> DeploymentHints {
    let mut provider = None;
    let mut strategy = None;
    if let Some(value) = annotations.get("greentic.deployment") {
        match value {
            JsonValue::String(s) => strategy = Some(s.to_string()),
            JsonValue::Object(map) => {
                if let Some(val) = map.get("provider").and_then(|v| v.as_str()) {
                    provider = Some(val.to_string());
                }
                if let Some(val) = map.get("strategy").and_then(|v| v.as_str()) {
                    strategy = Some(val.to_string());
                }
            }
            _ => {}
        }
    }

    DeploymentHints {
        provider: provider.unwrap_or_else(|| config.provider.as_str().to_string()),
        strategy: strategy.unwrap_or_else(|| config.strategy.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Action, DeployerConfig, Provider};
    use crate::iac::IaCTool;
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn builds_plan_from_example_pack() {
        let config = DeployerConfig {
            action: Action::Plan,
            provider: Provider::Aws,
            strategy: "iac-only".into(),
            tenant: "acme".into(),
            environment: "staging".into(),
            pack_path: PathBuf::from("examples/acme-pack"),
            yes: true,
            preview: false,
            dry_run: false,
            iac_tool: IaCTool::Terraform,
        };

        let plan = build_plan(&config).expect("should build plan");
        assert_eq!(plan.plan.tenant, "acme");
        assert_eq!(plan.plan.environment, "staging");
        assert_eq!(plan.plan.runners.len(), 1);
        assert_eq!(plan.plan.secrets.len(), 2);
        assert_eq!(plan.plan.oauth.len(), 2);
        assert_eq!(plan.deployment.provider, "aws");
        assert_eq!(plan.deployment.strategy, "iac-only");
    }

    #[test]
    fn builds_plan_from_complex_pack() {
        let config = DeployerConfig {
            action: Action::Plan,
            provider: Provider::Azure,
            strategy: "iac-only".into(),
            tenant: "acmeplus".into(),
            environment: "staging".into(),
            pack_path: PathBuf::from("examples/acme-plus-pack"),
            yes: true,
            preview: false,
            dry_run: false,
            iac_tool: IaCTool::Terraform,
        };

        let plan = build_plan(&config).expect("should build complex plan");
        assert_eq!(plan.plan.tenant, "acmeplus");
        assert!(plan.plan.messaging.is_some(), "expected messaging subjects");
        assert!(
            plan.plan.secrets.len() >= 4,
            "expected merged secrets from annotations and components"
        );
        assert!(
            plan.plan.channels.len() >= 2,
            "expected channel entries from connectors"
        );
        assert_eq!(plan.plan.oauth.len(), 2);
        assert_eq!(plan.deployment.provider, "azure");
        assert_eq!(plan.deployment.strategy, "iac-only");
    }

    #[test]
    fn deployment_hints_respect_annotations() {
        let mut annotations = JsonMap::new();
        annotations.insert(
            "greentic.deployment".into(),
            json!({ "provider": "k8s", "strategy": "kubectl" }),
        );
        let config = DeployerConfig {
            action: Action::Plan,
            provider: Provider::Aws,
            strategy: "iac-only".into(),
            tenant: "acme".into(),
            environment: "dev".into(),
            pack_path: PathBuf::from("examples/acme-pack"),
            yes: true,
            preview: false,
            dry_run: false,
            iac_tool: IaCTool::Terraform,
        };
        let hints = super::build_deployment_hints(&annotations, &config);
        assert_eq!(hints.provider, "k8s");
        assert_eq!(hints.strategy, "kubectl");
    }
}
