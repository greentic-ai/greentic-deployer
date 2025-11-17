use std::env;
use std::fmt::Write;
use std::fs;
use std::path::PathBuf;

use async_trait::async_trait;
use serde_json;
use tracing::info;

use crate::config::{DeployerConfig, Provider};
use crate::error::Result;
use crate::plan::{PlanContext, SecretContext};
use greentic_types::deployment::RunnerPlan;

const DEFAULT_CPU_MILLIS: u32 = 512;
const DEFAULT_MEMORY_MB: u32 = 1024;
use crate::providers::{ApplyManifest, ProviderArtifacts, ProviderBackend, ResolvedSecret};

/// AWS-specific backend.
#[derive(Clone)]
pub struct AwsBackend {
    config: DeployerConfig,
    plan: PlanContext,
}

impl AwsBackend {
    pub fn new(config: DeployerConfig, plan: PlanContext) -> Self {
        Self { config, plan }
    }

    fn render_main_tf(&self) -> Result<String> {
        let mut buffer = String::new();
        writeln!(
            &mut buffer,
            "# Terraform snippet for tenant `{}` in `{}`",
            self.config.tenant, self.config.environment
        )
        .ok();
        writeln!(
            &mut buffer,
            "terraform {{\n  backend \"local\" {{\n    path = \"terraform.tfstate\"\n  }}\n}}\n"
        )
        .ok();
        writeln!(
            &mut buffer,
            "provider \"aws\" {{\n  region = \"{}\"\n}}\n",
            self.region()
        )
        .ok();

        writeln!(&mut buffer, "locals {{").ok();
        writeln!(
            &mut buffer,
            "  nats_cluster = \"{}\"",
            Self::escape_value(&self.plan.messaging.logical_cluster)
        )
        .ok();
        writeln!(
            &mut buffer,
            "  nats_admin_url = \"{}\"",
            Self::escape_value(&self.plan.messaging.admin_url)
        )
        .ok();
        writeln!(
            &mut buffer,
            "  telemetry_endpoint = \"{}\"",
            Self::escape_value(&self.plan.telemetry.otlp_endpoint)
        )
        .ok();
        writeln!(&mut buffer, "}}\n").ok();

        buffer.push_str(&self.secret_data_blocks());
        buffer.push_str(&self.runner_resources());
        buffer.push_str(&self.channel_comments());
        buffer.push_str(&self.oauth_comments());

        Ok(buffer)
    }

    fn render_variables_tf(&self) -> String {
        let mut buffer = String::new();
        writeln!(
            &mut buffer,
            "variable \"aws_region\" {{\n  type = string\n  default = \"{}\"\n}}\n",
            self.region()
        )
        .ok();
        writeln!(
            &mut buffer,
            "variable \"otel_exporter_otlp_endpoint\" {{\n  type = string\n  default = \"{}\"\n}}\n",
            Self::escape_value(&self.plan.telemetry.otlp_endpoint)
        )
        .ok();

        if !self.plan.secrets.is_empty() {
            writeln!(&mut buffer, "# Secrets resolved via greentic-secrets").ok();
            for spec in &self.plan.secrets {
                let variable = self.secret_variable_name(spec);
                writeln!(
                    &mut buffer,
                    "variable \"{}\" {{\n  type = string\n  description = \"Secret identifier for {}\"\n}}\n",
                    variable,
                    spec.key
                )
                .ok();
            }
        }

        buffer
    }

    fn region(&self) -> String {
        env::var("AWS_REGION").unwrap_or_else(|_| "us-west-2".to_string())
    }

    fn sanitized_name(name: &str) -> String {
        let stripped: String = name
            .to_ascii_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        if stripped.is_empty() {
            "greentic".to_string()
        } else {
            stripped
        }
    }

    fn secret_variable_name(&self, spec: &SecretContext) -> String {
        format!("{}_secret_id", Self::sanitized_name(&spec.key))
    }

    fn secret_data_name(&self, spec: &SecretContext) -> String {
        format!("secret_{}", Self::sanitized_name(&spec.key))
    }

    fn runner_resource_name(&self, runner: &RunnerPlan) -> String {
        format!("runner_{}", Self::sanitized_name(&runner.name))
    }

    fn secret_data_blocks(&self) -> String {
        if self.plan.secrets.is_empty() {
            return String::new();
        }

        let mut block = String::new();
        writeln!(
            &mut block,
            "\n# Secret data sources (values resolved during apply via greentic-secrets)"
        )
        .ok();
        for spec in &self.plan.secrets {
            let data_name = self.secret_data_name(spec);
            let variable = self.secret_variable_name(spec);
            writeln!(
                &mut block,
                "data \"aws_secretsmanager_secret_version\" \"{}\" {{",
                data_name
            )
            .ok();
            writeln!(&mut block, "  secret_id = var.{}", variable).ok();
            writeln!(&mut block, "}}\n").ok();
        }

        block
    }

    fn runner_env_entries(&self, _runner: &RunnerPlan) -> Vec<String> {
        let mut entries = Vec::new();
        entries.push(format!(
            "    {{ \"name\": \"NATS_URL\", \"value\": \"{}\" }}",
            Self::escape_value(&self.plan.messaging.admin_url)
        ));
        entries.push(format!(
            "    {{ \"name\": \"OTEL_EXPORTER_OTLP_ENDPOINT\", \"value\": \"{}\" }}",
            Self::escape_value(&self.plan.telemetry.otlp_endpoint)
        ));
        let telemetry_attrs = self.telemetry_attributes();
        if !telemetry_attrs.is_empty() {
            entries.push(format!(
                "    {{ \"name\": \"OTEL_RESOURCE_ATTRIBUTES\", \"value\": \"{}\" }}",
                Self::escape_value(&telemetry_attrs)
            ));
        }

        for spec in &self.plan.secrets {
            let data_name = self.secret_data_name(spec);
            entries.push(format!(
                "    {{ \"name\": \"{}\", \"value\": data.aws_secretsmanager_secret_version.{}.secret_string }}",
                spec.key, data_name
            ));
        }

        entries
    }

    fn runner_resources(&self) -> String {
        let mut block = String::new();
        if self.plan.plan.runners.is_empty() {
            writeln!(
                &mut block,
                "\n# No runner services found in the plan; add components to execute greentic flows."
            )
            .ok();
            return block;
        }

        writeln!(
            &mut block,
            "resource \"aws_ecs_cluster\" \"nats\" {{\n  name = local.nats_cluster\n}}\n"
        )
        .ok();

        for runner in &self.plan.plan.runners {
            let resource_name = self.runner_resource_name(runner);
            let container_name = Self::escape_value(&runner.name);
            let env_block = self.runner_env_entries(runner).join(",\n");

            writeln!(
                &mut block,
                "resource \"aws_ecs_task_definition\" \"{}\" {{",
                resource_name
            )
            .ok();
            writeln!(&mut block, "  family = \"{}\"", container_name).ok();
            writeln!(&mut block, "  cpu = \"{}\"", DEFAULT_CPU_MILLIS).ok();
            writeln!(&mut block, "  memory = \"{}\"", DEFAULT_MEMORY_MB).ok();
            writeln!(
                &mut block,
                "  requires_compatibilities = [\"FARGATE\"]\n  network_mode = \"awsvpc\""
            )
            .ok();
            writeln!(&mut block, "  container_definitions = <<EOF").ok();
            writeln!(&mut block, "[ {{").ok();
            writeln!(&mut block, "  \"name\": \"{}\",", container_name).ok();
            writeln!(
                &mut block,
                "  \"image\": \"greentic/runner:latest\",\n  \"environment\": ["
            )
            .ok();
            writeln!(&mut block, "{}", env_block).ok();
            writeln!(&mut block, "  ]").ok();
            writeln!(&mut block, "}} ]").ok();
            writeln!(&mut block, "EOF\n}}\n").ok();

            writeln!(
                &mut block,
                "resource \"aws_ecs_service\" \"{}_service\" {{",
                resource_name
            )
            .ok();
            writeln!(&mut block, "  name = \"{}-service\"", container_name).ok();
            writeln!(&mut block, "  cluster = aws_ecs_cluster.nats.id").ok();
            writeln!(
                &mut block,
                "  task_definition = aws_ecs_task_definition.{}.arn",
                resource_name
            )
            .ok();
            writeln!(&mut block, "  desired_count = 1").ok();
            writeln!(&mut block, "}}\n").ok();
        }

        block
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
                "# - /oauth/{}/callback via {}",
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

    fn escape_value(value: &str) -> String {
        value.replace('\\', "\\\\").replace('"', "\\\"")
    }
}

#[async_trait]
impl ProviderBackend for AwsBackend {
    async fn plan(&self) -> Result<ProviderArtifacts> {
        let main_tf = self.render_main_tf()?;
        let variables_tf = self.render_variables_tf();
        let plan_json = serde_json::to_string_pretty(&self.plan)?;

        Ok(
            ProviderArtifacts::named(
                Provider::Aws,
                format!(
                    "AWS deployment for tenant {} in {}",
                    self.config.tenant, self.config.environment
                ),
                self.plan.clone(),
            )
            .with_file("master.tf", main_tf)
            .with_file("variables.tf", variables_tf)
            .with_file("plan.json", plan_json),
        )
    }

    async fn apply(&self, artifacts: &ProviderArtifacts, secrets: &[ResolvedSecret]) -> Result<()> {
        self.persist_manifest("apply", artifacts, secrets)?;
        info!(
            "applying AWS deployment for tenant={} env={} (manifest: {})",
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
            "destroying AWS deployment for tenant={} env={} (manifest: {})",
            self.config.tenant,
            self.config.environment,
            self.manifest_path("destroy").display()
        );
        Ok(())
    }
}

impl AwsBackend {
    fn deploy_base(&self) -> PathBuf {
        PathBuf::from("deploy")
            .join(self.config.provider.as_str())
            .join(&self.config.tenant)
            .join(&self.config.environment)
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
