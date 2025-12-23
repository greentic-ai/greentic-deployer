use std::fmt::Write;
use std::fs;
use std::path::PathBuf;

use async_trait::async_trait;
use tracing::info;

use crate::config::{DeployerConfig, Provider};
use crate::error::Result;
use crate::plan::{PlanContext, requirement_scope};
use crate::providers::{ApplyManifest, ProviderArtifacts, ProviderBackend, ResolvedSecret};
use greentic_types::deployment::RunnerPlan;
use greentic_types::secrets::SecretRequirement;

fn runner_cpu_millis(runner: &RunnerPlan) -> u32 {
    runner
        .capabilities
        .get("cpu_millis")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(500)
}

fn runner_memory_mb(runner: &RunnerPlan) -> u32 {
    runner
        .capabilities
        .get("memory_mb")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(1024)
}

fn cpu_to_k8s(millis: u32) -> String {
    format!("{}m", millis)
}

fn memory_to_k8s(mb: u32) -> String {
    format!("{}Mi", mb)
}

/// GCP-specific backend.
#[derive(Clone)]
pub struct GcpBackend {
    config: DeployerConfig,
    plan: PlanContext,
}

impl GcpBackend {
    pub fn new(config: DeployerConfig, plan: PlanContext) -> Self {
        Self { config, plan }
    }

    fn is_external_component(&self, runner: &RunnerPlan) -> bool {
        self.plan
            .external_components
            .iter()
            .any(|id| id == &runner.name)
    }

    fn render_main_yaml(&self) -> String {
        let mut docs = String::new();
        writeln!(&mut docs, "resources:").ok();

        if self.plan.plan.runners.is_empty() {
            writeln!(
                &mut docs,
                "  # no runner services detected; add Greentic components to this plan"
            )
            .ok();
        } else {
            for runner in &self.plan.plan.runners {
                let resource_name = format!("{}-runner", Self::sanitize_name(&runner.name));
                if self.is_external_component(runner) {
                    writeln!(&mut docs, "  # external-facing component").ok();
                }
                writeln!(&mut docs, "  - name: {}", resource_name).ok();
                writeln!(&mut docs, "    type: run.v1.service").ok();
                writeln!(&mut docs, "    properties:").ok();
                writeln!(&mut docs, "      template:").ok();
                writeln!(&mut docs, "        spec:").ok();
                writeln!(&mut docs, "          containers:").ok();
                writeln!(
                    &mut docs,
                    "          - name: {}",
                    Self::sanitize_name(&runner.name)
                )
                .ok();
                writeln!(
                    &mut docs,
                    "            image: gcr.io/greentic/runner:latest"
                )
                .ok();
                writeln!(&mut docs, "            env:").ok();
                for env in self.gcp_env_entries(runner) {
                    writeln!(&mut docs, "{}", env).ok();
                }
                let cpu = cpu_to_k8s(runner_cpu_millis(runner));
                let memory = memory_to_k8s(runner_memory_mb(runner));
                writeln!(&mut docs, "            resources:").ok();
                writeln!(&mut docs, "              limits:").ok();
                writeln!(&mut docs, "                cpu: {}", cpu).ok();
                writeln!(&mut docs, "                memory: {}", memory).ok();
                writeln!(&mut docs, "        scaling:").ok();
                writeln!(
                    &mut docs,
                    "          minInstanceCount: {}",
                    runner.replicas.max(1)
                )
                .ok();
                writeln!(
                    &mut docs,
                    "          maxInstanceCount: {}",
                    (runner.replicas + 1).max(1)
                )
                .ok();
                writeln!(
                    &mut docs,
                    "          ingress: {}",
                    if self.is_external_component(runner) {
                        "INGRESS_TRAFFIC_ALL"
                    } else {
                        "INGRESS_TRAFFIC_INTERNAL_ONLY"
                    }
                )
                .ok();
            }
        }

        docs.push_str(&self.channel_comments());
        docs.push_str(&self.oauth_comments());

        docs
    }

    fn render_parameters_yaml(&self) -> String {
        let mut docs = String::new();
        writeln!(&mut docs, "secret_paths:").ok();
        if self.plan.secrets.is_empty() {
            writeln!(&mut docs, "  # no secrets defined in plan").ok();
        } else {
            for spec in &self.plan.secrets {
                writeln!(
                    &mut docs,
                    "  {}: {}",
                    spec.key.as_str(),
                    self.secret_manager_path(spec)
                )
                .ok();
            }
        }
        writeln!(
            &mut docs,
            "nats_admin_url: {}",
            Self::yaml_quoted(&self.plan.messaging.admin_url)
        )
        .ok();
        writeln!(
            &mut docs,
            "telemetry_endpoint: {}",
            Self::yaml_quoted(&self.plan.telemetry.otlp_endpoint)
        )
        .ok();
        docs
    }

    fn secret_manager_path(&self, spec: &SecretRequirement) -> String {
        let scope = requirement_scope(spec, &self.plan.plan.environment, &self.plan.plan.tenant);
        let team = scope.team.clone().unwrap_or_else(|| "_".to_string());
        format!(
            "projects/greentic/secrets/greentic-{}-{}-{}-{}/versions/latest",
            scope.tenant,
            scope.env,
            team,
            spec.key.as_str()
        )
    }

    fn gcp_env_entries(&self, _runner: &RunnerPlan) -> Vec<String> {
        let mut entries = Vec::new();
        entries.push(format!(
            "            - name: NATS_URL\n              value: {}",
            Self::yaml_quoted(&self.plan.messaging.admin_url)
        ));
        entries.push(format!(
            "            - name: OTEL_EXPORTER_OTLP_ENDPOINT\n              value: {}",
            Self::yaml_quoted(&self.plan.telemetry.otlp_endpoint)
        ));
        let telemetry_attrs = self.telemetry_attributes();
        if !telemetry_attrs.is_empty() {
            entries.push(format!(
                "            - name: OTEL_RESOURCE_ATTRIBUTES\n              value: {}",
                Self::yaml_quoted(&telemetry_attrs)
            ));
        }
        for channel in &self.plan.channels {
            let var = format!(
                "CHANNEL_{}_INGRESS",
                Self::sanitize_name(&channel.name).to_ascii_uppercase()
            );
            let value = channel.ingress.join(",");
            entries.push(format!(
                "            - name: {}\n              value: {}",
                var,
                Self::yaml_quoted(&value)
            ));
        }

        for spec in &self.plan.secrets {
            entries.push(format!(
                "            - name: {}\n              valueFrom:\n                secretKeyRef:\n                  secret: {}\n                  version: latest",
                spec.key.as_str(),
                self.secret_manager_path(spec)
            ));
        }

        entries
    }

    fn channel_comments(&self) -> String {
        if self.plan.channels.is_empty() {
            return String::new();
        }
        let mut block = String::new();
        writeln!(&mut block, "\n# Channel ingress endpoints").ok();
        for channel in &self.plan.channels {
            let ingress = channel.ingress.join(", ");
            writeln!(
                &mut block,
                "# - {} (type = {}, oauth_required = {})",
                channel.name, channel.kind, channel.oauth_required
            )
            .ok();
            writeln!(&mut block, "#   ingress: {}", ingress).ok();
        }
        block
    }

    fn oauth_comments(&self) -> String {
        if self.plan.plan.oauth.is_empty() {
            return String::new();
        }
        let mut block = String::new();
        writeln!(&mut block, "\n# OAuth redirect URLs").ok();
        for client in &self.plan.plan.oauth {
            writeln!(
                &mut block,
                "# - /oauth/{}/callback -> {}",
                client.provider_id, client.redirect_path
            )
            .ok();
        }
        block
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

    fn yaml_quoted(value: &str) -> String {
        format!("'{}'", value.replace('\'', "''"))
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
impl ProviderBackend for GcpBackend {
    async fn plan(&self) -> Result<ProviderArtifacts> {
        let yaml = self.render_main_yaml();
        let parameters = self.render_parameters_yaml();
        let plan_json = serde_json::to_string_pretty(&self.plan)?;

        Ok(ProviderArtifacts::named(
            Provider::Gcp,
            format!(
                "GCP deployment for tenant {} in {}",
                self.config.tenant, self.config.environment
            ),
            self.plan.clone(),
        )
        .with_file("master.yaml", yaml)
        .with_file("parameters.yaml", parameters)
        .with_file("plan.json", plan_json))
    }

    async fn apply(&self, artifacts: &ProviderArtifacts, secrets: &[ResolvedSecret]) -> Result<()> {
        self.persist_manifest("apply", artifacts, secrets)?;
        info!(
            "applying GCP deployment for tenant={} env={} (manifest: {})",
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
            "destroying GCP deployment for tenant={} env={} (manifest: {})",
            self.config.tenant,
            self.config.environment,
            self.manifest_path("destroy").display()
        );
        Ok(())
    }
}

impl GcpBackend {
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
}
