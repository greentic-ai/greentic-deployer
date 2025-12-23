use std::fmt::Write;
use std::fs;
use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::{self, json};
use tracing::info;

use crate::config::{DeployerConfig, Provider};
use crate::error::Result;
use crate::plan::{PlanContext, requirement_scope};
use crate::providers::{ApplyManifest, ProviderArtifacts, ProviderBackend, ResolvedSecret};
use greentic_types::deployment::RunnerPlan;
use greentic_types::secrets::SecretRequirement;

fn runner_cpu_cores(runner: &RunnerPlan) -> String {
    let millis = runner
        .capabilities
        .get("cpu_millis")
        .and_then(|v| v.as_u64())
        .unwrap_or(500);
    format!("{:.2}", (millis as f64) / 1000.0)
}

fn runner_memory_gib(runner: &RunnerPlan) -> String {
    let mb = runner
        .capabilities
        .get("memory_mb")
        .and_then(|v| v.as_u64())
        .unwrap_or(1024);
    format!("{:.2}Gi", (mb as f64) / 1024.0)
}

/// Azure-specific backend stub.
#[derive(Clone)]
pub struct AzureBackend {
    config: DeployerConfig,
    plan: PlanContext,
}

impl AzureBackend {
    pub fn new(config: DeployerConfig, plan: PlanContext) -> Self {
        Self { config, plan }
    }

    fn is_external_component(&self, runner: &RunnerPlan) -> bool {
        self.plan
            .external_components
            .iter()
            .any(|id| id == &runner.name)
    }

    fn render_main_bicep(&self) -> String {
        let mut body = String::new();
        writeln!(
            &mut body,
            "// Azure Bicep for tenant {} ({})",
            self.config.tenant, self.config.environment
        )
        .ok();
        writeln!(
            &mut body,
            "param tenant string = '{}'",
            Self::bicep_escape(&self.config.tenant)
        )
        .ok();
        writeln!(
            &mut body,
            "param environment string = '{}'",
            Self::bicep_escape(&self.config.environment)
        )
        .ok();
        writeln!(
            &mut body,
            "param telemetryEndpoint string = '{}'",
            Self::bicep_escape(&self.plan.telemetry.otlp_endpoint)
        )
        .ok();
        writeln!(
            &mut body,
            "param natsAdminUrl string = '{}'",
            Self::bicep_escape(&self.plan.messaging.admin_url)
        )
        .ok();
        writeln!(&mut body, "param secretPaths object = {{}}").ok();
        writeln!(
            &mut body,
            "var deploymentName = '\\${{tenant}}-\\${{environment}}'"
        )
        .ok();
        writeln!(
            &mut body,
            "var telemetryAttributes = '{}'",
            Self::bicep_escape(&self.telemetry_attributes())
        )
        .ok();

        if self.plan.plan.runners.is_empty() {
            writeln!(&mut body, "\n// No runners were inferred from the pack.").ok();
        } else {
            for runner in &self.plan.plan.runners {
                let resource = format!("runner{}", Self::sanitize_name(&runner.name));
                let env_block = self.azure_env_entries(runner).join("\n");
                let secrets_block = if self.plan.secrets.is_empty() {
                    "      secrets: []\n".to_string()
                } else {
                    let mut secrets = String::new();
                    secrets.push_str("      secrets:\n      [\n");
                    for spec in &self.plan.secrets {
                        let key = spec.key.as_str();
                        secrets.push_str(&format!(
                            "        {{ name: '{}', value: secretPaths['{}'] }}\n",
                            key, key
                        ));
                    }
                    secrets.push_str("      ]\n");
                    secrets
                };

                writeln!(
                    &mut body,
                    "\n{}resource {} 'Microsoft.Web/containerApps@2023-08-01' = {{",
                    if self.is_external_component(runner) {
                        "// External-facing component\n"
                    } else {
                        ""
                    },
                    resource
                )
                .ok();
                writeln!(
                    &mut body,
                    "  name: '${{deploymentName}}-{}'",
                    Self::sanitize_name(&runner.name)
                )
                .ok();
                writeln!(&mut body, "  location: resourceGroup().location").ok();
                writeln!(&mut body, "  properties: {{").ok();
                writeln!(&mut body, "    configuration: {{").ok();
                body.push_str(&secrets_block);
                if self.is_external_component(runner) {
                    writeln!(
                        &mut body,
                        "      ingress: {{ external: true, targetPort: 8080, transport: 'auto' }}"
                    )
                    .ok();
                }
                writeln!(&mut body, "    }}").ok();
                writeln!(&mut body, "    template: {{").ok();
                let min = runner.replicas.max(1);
                let max = (runner.replicas + 1).max(min);
                writeln!(
                    &mut body,
                    "      scale: {{ minReplicas: {}, maxReplicas: {} }}",
                    min, max
                )
                .ok();
                writeln!(&mut body, "      containers: [").ok();
                writeln!(&mut body, "        {{").ok();
                writeln!(
                    &mut body,
                    "          name: '{}'",
                    Self::sanitize_name(&runner.name)
                )
                .ok();
                writeln!(&mut body, "          image: 'greentic/runner:latest'").ok();
                writeln!(&mut body, "          env: [").ok();
                writeln!(&mut body, "{}", env_block).ok();
                writeln!(&mut body, "          ]").ok();
                writeln!(
                    &mut body,
                    "          resources: {{ requests: {{ cpu: '{}', memory: '{}' }} }}",
                    runner_cpu_cores(runner),
                    runner_memory_gib(runner)
                )
                .ok();
                writeln!(&mut body, "        }}").ok();
                writeln!(&mut body, "      ]").ok();
                writeln!(&mut body, "    }}").ok();
                writeln!(&mut body, "  }}").ok();
                writeln!(&mut body, "}}\n").ok();
            }
        }

        body.push_str(&self.channel_comments());
        body.push_str(&self.oauth_comments());

        body
    }

    fn render_parameters(&self) -> String {
        let mut parameters = serde_json::Map::new();
        parameters.insert("tenant".to_string(), json!({ "value": self.config.tenant }));
        parameters.insert(
            "environment".to_string(),
            json!({ "value": self.config.environment }),
        );
        parameters.insert(
            "telemetryEndpoint".to_string(),
            json!({ "value": self.plan.telemetry.otlp_endpoint }),
        );
        parameters.insert(
            "natsAdminUrl".to_string(),
            json!({ "value": self.plan.messaging.admin_url }),
        );

        let secret_map = self.secret_paths_map();
        parameters.insert("secretPaths".to_string(), json!({ "value": secret_map }));

        let payload = json!({
            "$schema": "https://schema.management.azure.com/schemas/2019-04-01/deploymentParameters.json#",
            "contentVersion": "1.0.0.0",
            "parameters": parameters,
        });

        serde_json::to_string_pretty(&payload).unwrap_or_default()
    }

    fn secret_paths_map(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut secrets = serde_json::Map::new();
        for spec in &self.plan.secrets {
            secrets.insert(
                spec.key.as_str().to_string(),
                json!(self.secret_reference_path(spec)),
            );
        }
        secrets
    }

    fn secret_reference_path(&self, spec: &SecretRequirement) -> String {
        let scope = requirement_scope(spec, &self.plan.plan.environment, &self.plan.plan.tenant);
        format!(
            "@sec:greentic/{}/{}/{}/{}",
            scope.env,
            scope.tenant,
            scope.team.unwrap_or_else(|| "_".to_string()),
            spec.key.as_str()
        )
    }

    fn azure_env_entries(&self, _runner: &RunnerPlan) -> Vec<String> {
        let mut entries = Vec::new();
        entries.push("          { name: 'NATS_URL', value: natsAdminUrl }".to_string());
        entries.push(
            "          { name: 'OTEL_EXPORTER_OTLP_ENDPOINT', value: telemetryEndpoint }"
                .to_string(),
        );
        let attrs = self.telemetry_attributes();
        if !attrs.is_empty() {
            entries.push(format!(
                "          {{ name: 'OTEL_RESOURCE_ATTRIBUTES', value: '{}' }}",
                Self::bicep_escape(&attrs)
            ));
        }
        for channel in &self.plan.channels {
            let var = format!(
                "CHANNEL_{}_INGRESS",
                Self::sanitize_name(&channel.name).to_ascii_uppercase()
            );
            let value = channel.ingress.join(",");
            entries.push(format!(
                "          {{ name: '{}', value: '{}' }}",
                var,
                Self::bicep_escape(&value)
            ));
        }

        for spec in &self.plan.secrets {
            entries.push(format!(
                "          {{ name: '{}', secretRef: '{}' }}",
                spec.key.as_str(),
                spec.key.as_str()
            ));
        }

        entries
    }

    fn channel_comments(&self) -> String {
        if self.plan.channels.is_empty() {
            return String::new();
        }
        let mut block = String::new();
        writeln!(&mut block, "\n// Channel ingress endpoints").ok();
        for channel in &self.plan.channels {
            let ingress = channel.ingress.join(", ");
            writeln!(
                &mut block,
                "// - {} (type = {}, oauth_required = {})",
                channel.name, channel.kind, channel.oauth_required
            )
            .ok();
            writeln!(&mut block, "//   ingress: {}", ingress).ok();
        }
        block
    }

    fn oauth_comments(&self) -> String {
        if self.plan.plan.oauth.is_empty() {
            return String::new();
        }
        let mut block = String::new();
        writeln!(&mut block, "\n// OAuth redirect URLs").ok();
        for client in &self.plan.plan.oauth {
            writeln!(
                &mut block,
                "// - /oauth/{}/callback -> {}",
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

    fn sanitize_name(value: &str) -> String {
        value
            .to_ascii_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect()
    }

    fn bicep_escape(value: &str) -> String {
        value.replace('\'', "''")
    }
}

#[async_trait]
impl ProviderBackend for AzureBackend {
    async fn plan(&self) -> Result<ProviderArtifacts> {
        let bicep = self.render_main_bicep();
        let parameters = self.render_parameters();
        let plan_json = serde_json::to_string_pretty(&self.plan)?;

        Ok(ProviderArtifacts::named(
            Provider::Azure,
            format!(
                "Azure deployment for tenant {} in {}",
                self.config.tenant, self.config.environment
            ),
            self.plan.clone(),
        )
        .with_file("master.bicep", bicep)
        .with_file("parameters.json", parameters)
        .with_file("plan.json", plan_json))
    }

    async fn apply(&self, artifacts: &ProviderArtifacts, secrets: &[ResolvedSecret]) -> Result<()> {
        self.persist_manifest("apply", artifacts, secrets)?;
        info!(
            "applying Azure deployment for tenant={} env={} (manifest: {})",
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
            "destroying Azure deployment for tenant={} env={} (manifest: {})",
            self.config.tenant,
            self.config.environment,
            self.manifest_path("destroy").display()
        );
        Ok(())
    }
}

impl AzureBackend {
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
