use std::fs;
use std::path::PathBuf;

use async_trait::async_trait;
use serde_json;
use tracing::info;

use crate::config::{DeployerConfig, Provider};
use crate::error::Result;
use crate::plan::PlanContext;
use crate::plan::requirement_scope;
use crate::providers::{ApplyManifest, ProviderArtifacts, ProviderBackend, ResolvedSecret};

/// Placeholder backend for Local deployments when no deployment executor is registered.
#[derive(Clone)]
pub struct LocalBackend {
    config: DeployerConfig,
    plan: PlanContext,
}

impl LocalBackend {
    pub fn new(config: DeployerConfig, plan: PlanContext) -> Self {
        Self { config, plan }
    }

    fn compose_yaml(&self) -> String {
        let mut doc = String::new();
        doc.push_str("version: \"3.9\"\nservices:\n");
        for runner in &self.plan.plan.runners {
            let name = Self::sanitize_name(&runner.name);
            doc.push_str(&format!("  {}:\n", name));
            doc.push_str("    image: greentic/runner:latest\n");
            doc.push_str("    environment:\n");
            for env in self.env_entries() {
                doc.push_str(&format!("      - {}\n", env));
            }
            for spec in &self.plan.secrets {
                let scope =
                    requirement_scope(spec, &self.plan.plan.environment, &self.plan.plan.tenant);
                let var = spec.key.as_str();
                doc.push_str(&format!(
                    "      - {}=@sec:greentic/{}/{}/{}/{}\n",
                    var,
                    scope.env,
                    scope.tenant,
                    scope.team.clone().unwrap_or_else(|| "_".to_string()),
                    spec.key.as_str(),
                ));
            }
        }
        doc
    }

    fn env_entries(&self) -> Vec<String> {
        let mut entries = Vec::new();
        entries.push(format!("NATS_URL={}", self.plan.messaging.admin_url));
        entries.push(format!(
            "OTEL_EXPORTER_OTLP_ENDPOINT={}",
            self.plan.telemetry.otlp_endpoint
        ));
        let attrs = self.telemetry_attributes();
        if !attrs.is_empty() {
            entries.push(format!("OTEL_RESOURCE_ATTRIBUTES={}", attrs));
        }
        for channel in &self.plan.channels {
            let var = format!(
                "CHANNEL_{}_INGRESS",
                Self::sanitize_name(&channel.name).to_ascii_uppercase()
            );
            entries.push(format!("{}={}", var, channel.ingress.join(",")));
        }
        entries
    }

    fn deploy_base(&self) -> PathBuf {
        self.config.provider_output_dir()
    }

    fn manifest_path(&self, stage: &str) -> PathBuf {
        self.deploy_base().join(format!("{stage}-manifest.json"))
    }

    fn persist_manifest(
        &self,
        stage: &str,
        artifacts: &ProviderArtifacts,
        secrets: &[ResolvedSecret],
    ) -> Result<()> {
        let manifest = ApplyManifest::build(stage, &self.config, artifacts, secrets);
        let path = self.manifest_path(stage);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let payload = serde_json::to_string_pretty(&manifest)?;
        fs::write(&path, payload)?;
        Ok(())
    }

    fn info_note(&self) -> String {
        "Local deployments require a deployment pack executor; this backend emits compose.yaml and manifests for inspection."
            .to_string()
    }

    fn telemetry_attributes(&self) -> String {
        self.plan
            .telemetry
            .resource_attributes
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(",")
    }

    fn sanitize_name(value: &str) -> String {
        value
            .to_ascii_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect()
    }
}

#[async_trait]
impl ProviderBackend for LocalBackend {
    async fn plan(&self) -> Result<ProviderArtifacts> {
        let note = self.info_note();
        let plan_json = serde_json::to_string_pretty(&self.plan)?;
        Ok(ProviderArtifacts::named(
            Provider::Local,
            format!(
                "Local deployment for tenant {} in {}",
                self.config.tenant, self.config.environment
            ),
            self.plan.clone(),
        )
        .with_file("plan.json", plan_json)
        .with_file("compose.yaml", self.compose_yaml())
        .with_file("README.txt", note))
    }

    async fn apply(&self, artifacts: &ProviderArtifacts, secrets: &[ResolvedSecret]) -> Result<()> {
        self.persist_manifest("apply", artifacts, secrets)?;
        info!(
            "Local deployment recorded (executor required) for tenant={} env={} (manifest: {})",
            self.config.tenant,
            self.config.environment,
            self.manifest_path("apply").display()
        );
        Ok(())
    }

    async fn destroy(
        &self,
        artifacts: &ProviderArtifacts,
        secrets: &[ResolvedSecret],
    ) -> Result<()> {
        self.persist_manifest("destroy", artifacts, secrets)?;
        info!(
            "Local destroy recorded (executor required) for tenant={} env={} (manifest: {})",
            self.config.tenant,
            self.config.environment,
            self.manifest_path("destroy").display()
        );
        Ok(())
    }
}
