use std::fmt::Write;
use std::fs;
use std::path::PathBuf;

use async_trait::async_trait;
use serde_json;
use tracing::info;

use crate::config::{DeployerConfig, Provider};
use crate::error::Result;
use crate::plan::{PlanContext, requirement_scope};
use crate::providers::{ApplyManifest, ProviderArtifacts, ProviderBackend, ResolvedSecret};
use greentic_types::deployment::RunnerPlan;
use greentic_types::secrets::SecretRequirement;

/// Placeholder backend for K8s deployments when no deployment executor is registered.
#[derive(Clone)]
pub struct K8sBackend {
    config: DeployerConfig,
    plan: PlanContext,
}

impl K8sBackend {
    pub fn new(config: DeployerConfig, plan: PlanContext) -> Self {
        Self { config, plan }
    }

    fn render_k8s_yaml(&self) -> String {
        let mut docs = String::new();
        for runner in &self.plan.plan.runners {
            docs.push_str(&self.service_block(runner));
            docs.push_str(&self.deployment_block(runner));
            if self.is_external_component(runner) {
                docs.push_str(&self.ingress_block(runner));
            }
        }
        docs
    }

    fn is_external_component(&self, runner: &RunnerPlan) -> bool {
        self.plan
            .external_components
            .iter()
            .any(|id| id == &runner.name)
    }

    fn deployment_block(&self, runner: &RunnerPlan) -> String {
        let mut doc = String::new();
        let name = Self::sanitize_name(&runner.name);
        let _ = writeln!(&mut doc, "---");
        let _ = writeln!(&mut doc, "apiVersion: apps/v1");
        let _ = writeln!(&mut doc, "kind: Deployment");
        let _ = writeln!(&mut doc, "metadata:\n  name: {}", name);
        let _ = writeln!(&mut doc, "spec:\n  replicas: {}", runner.replicas.max(1));
        let _ = writeln!(
            &mut doc,
            "  selector:\n    matchLabels:\n      app: {}",
            name
        );
        let _ = writeln!(
            &mut doc,
            "  template:\n    metadata:\n      labels:\n        app: {}",
            name
        );
        let _ = writeln!(&mut doc, "    spec:\n      containers:");
        let _ = writeln!(&mut doc, "      - name: {}", name);
        let _ = writeln!(&mut doc, "        image: ghcr.io/greentic/runner:latest");
        let _ = writeln!(&mut doc, "        env:");
        for env in self.env_entries() {
            let _ = writeln!(&mut doc, "        - {}", env);
        }
        for spec in &self.plan.secrets {
            let _ = writeln!(
                &mut doc,
                "        - name: {}\n          value: {}\n",
                spec.key.as_str(),
                Self::yaml_quoted(&self.secret_reference(spec))
            );
        }
        writeln!(
            &mut doc,
            "        resources:\n          requests:\n            cpu: \"{}m\"\n            memory: \"{}Mi\"",
            runner
                .capabilities
                .get("cpu_millis")
                .and_then(|v| v.as_u64())
                .unwrap_or(500),
            runner
                .capabilities
                .get("memory_mb")
                .and_then(|v| v.as_u64())
                .unwrap_or(1024)
        )
        .ok();
        let _ = writeln!(&mut doc, "        ports:\n        - containerPort: 8080");
        doc
    }

    fn service_block(&self, runner: &RunnerPlan) -> String {
        let mut doc = String::new();
        let name = Self::sanitize_name(&runner.name);
        let _ = writeln!(&mut doc, "---");
        let _ = writeln!(&mut doc, "apiVersion: v1");
        let _ = writeln!(&mut doc, "kind: Service");
        let _ = writeln!(&mut doc, "metadata:\n  name: {}-svc", name);
        let _ = writeln!(&mut doc, "spec:\n  selector:\n    app: {}", name);
        let _ = writeln!(&mut doc, "  ports:\n  - port: 80\n    targetPort: 8080");
        doc
    }

    fn ingress_block(&self, runner: &RunnerPlan) -> String {
        let mut doc = String::new();
        let name = Self::sanitize_name(&runner.name);
        let host = self
            .plan
            .channels
            .first()
            .map(|c| c.ingress.first().cloned().unwrap_or_default())
            .unwrap_or_else(|| {
                format!(
                    "{}.{}.example.local",
                    self.config.environment, self.config.tenant
                )
            });
        let _ = writeln!(&mut doc, "---");
        let _ = writeln!(&mut doc, "apiVersion: networking.k8s.io/v1");
        let _ = writeln!(&mut doc, "kind: Ingress");
        let _ = writeln!(&mut doc, "metadata:\n  name: {}-ingress", name);
        let _ = writeln!(&mut doc, "spec:\n  rules:\n  - host: {}", host);
        let _ = writeln!(&mut doc, "    http:\n      paths:");
        let _ = writeln!(
            &mut doc,
            "      - path: /\n        pathType: Prefix\n        backend:\n          service:\n            name: {}-svc\n            port:\n              number: 80",
            name
        );
        doc
    }

    fn env_entries(&self) -> Vec<String> {
        let mut entries = Vec::new();
        entries.push(format!(
            "name: NATS_URL\n          value: {}",
            Self::yaml_quoted(&self.plan.messaging.admin_url)
        ));
        entries.push(format!(
            "name: OTEL_EXPORTER_OTLP_ENDPOINT\n          value: {}",
            Self::yaml_quoted(&self.plan.telemetry.otlp_endpoint)
        ));
        let attrs = self.telemetry_attributes();
        if !attrs.is_empty() {
            entries.push(format!(
                "name: OTEL_RESOURCE_ATTRIBUTES\n          value: {}",
                Self::yaml_quoted(&attrs)
            ));
        }
        for channel in &self.plan.channels {
            let var = format!(
                "CHANNEL_{}_INGRESS",
                Self::sanitize_name(&channel.name).to_ascii_uppercase()
            );
            entries.push(format!(
                "name: {}\n          value: {}",
                var,
                Self::yaml_quoted(&channel.ingress.join(","))
            ));
        }
        entries
    }

    fn secret_reference(&self, spec: &SecretRequirement) -> String {
        let scope = requirement_scope(spec, &self.plan.plan.environment, &self.plan.plan.tenant);
        format!(
            "@sec:greentic/{}/{}/{}/{}",
            scope.env,
            scope.tenant,
            scope.team.unwrap_or_else(|| "_".to_string()),
            spec.key.as_str()
        )
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
        "K8s deployments require a deployment pack executor; this backend emits k8s.yaml and manifests for inspection."
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

    fn yaml_quoted(value: &str) -> String {
        format!("'{}'", value.replace('\'', "''"))
    }
}

#[async_trait]
impl ProviderBackend for K8sBackend {
    async fn plan(&self) -> Result<ProviderArtifacts> {
        let note = self.info_note();
        let plan_json = serde_json::to_string_pretty(&self.plan)?;
        let k8s_yaml = self.render_k8s_yaml();
        Ok(ProviderArtifacts::named(
            Provider::K8s,
            format!(
                "K8s deployment for tenant {} in {}",
                self.config.tenant, self.config.environment
            ),
            self.plan.clone(),
        )
        .with_file("plan.json", plan_json)
        .with_file("k8s.yaml", k8s_yaml)
        .with_file("README.txt", note))
    }

    async fn apply(&self, artifacts: &ProviderArtifacts, secrets: &[ResolvedSecret]) -> Result<()> {
        self.persist_manifest("apply", artifacts, secrets)?;
        info!(
            "K8s deployment recorded (executor required) for tenant={} env={} (manifest: {})",
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
            "K8s destroy recorded (executor required) for tenant={} env={} (manifest: {})",
            self.config.tenant,
            self.config.environment,
            self.manifest_path("destroy").display()
        );
        Ok(())
    }
}
