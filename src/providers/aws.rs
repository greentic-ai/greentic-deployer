use std::fmt::Write;

use async_trait::async_trait;
use serde_json;
use tracing::info;

use crate::config::{DeployerConfig, Provider};
use crate::error::Result;
use crate::plan::DeploymentPlan;
use crate::providers::{ProviderArtifacts, ProviderBackend};

/// AWS-specific backend.
#[derive(Clone)]
pub struct AwsBackend {
    config: DeployerConfig,
    plan: DeploymentPlan,
}

impl AwsBackend {
    pub fn new(config: DeployerConfig, plan: DeploymentPlan) -> Self {
        Self { config, plan }
    }

    fn terraform_body(&self) -> Result<String> {
        let mut buffer = String::new();
        writeln!(
            &mut buffer,
            "# Terraform snippet for tenant `{}` in `{}`",
            self.config.tenant, self.config.environment
        )
        .ok();
        writeln!(&mut buffer, "provider \"aws\" {{ region = \"us-west-2\" }}").ok();
        writeln!(
            &mut buffer,
            "locals {{ nats = \"{}\" replicas = {} }}",
            self.plan.messaging.nats.cluster_name, self.plan.messaging.nats.replicas
        )
        .ok();
        writeln!(&mut buffer, "# Runners ({})", self.plan.runners.len()).ok();

        for runner in &self.plan.runners {
            writeln!(
                &mut buffer,
                "resource \"aws_ecs_task_definition\" \"{}\" {{\n  family = \"{}\"\n  container_definitions = <<EOF\n  [ {{ \"name\": \"{}\", \"image\": \"greentic/runner:latest\", \"environment\": [\n{}  ] }} ]\n  EOF\n}}",
                runner.name.replace('@', "_"),
                runner.name,
                runner.name,
                runner.bindings
                    .iter()
                    .map(|binding| format!("    {{ \"name\": \"{}\", \"value\": \"{}\" }}", binding.name, binding.detail))
                    .collect::<Vec<_>>()
                    .join(",\n")
            )
            .ok();
        }

        writeln!(
            &mut buffer,
            "\n# Secrets referenced:\n{}",
            self.plan
                .secrets
                .iter()
                .map(|spec| format!(
                    "data \"aws_secretsmanager_secret\" \"{}\" {{ name = \"{}\" }}",
                    spec.name.to_ascii_lowercase(),
                    spec.name
                ))
                .collect::<Vec<_>>()
                .join("\n")
        )
        .ok();

        writeln!(
            &mut buffer,
            "\n# OAuth redirects:\n{}",
            self.plan
                .oauth_clients
                .iter()
                .flat_map(|client| client.redirect_urls.iter().cloned())
                .collect::<Vec<_>>()
                .join("\n")
        )
        .ok();

        writeln!(
            &mut buffer,
            "\n# Telemetry envs\nOTEL_EXPORTER_OTLP_ENDPOINT = \"{}\"",
            self.plan.telemetry.otlp_endpoint
        )
        .ok();

        Ok(buffer)
    }
}

#[async_trait]
impl ProviderBackend for AwsBackend {
    async fn plan(&self) -> Result<ProviderArtifacts> {
        let terraform = self.terraform_body()?;
        let plan_json = serde_json::to_string_pretty(&self.plan)?;

        let artifacts = ProviderArtifacts::named(
            Provider::Aws,
            format!(
                "AWS deployment for tenant {} in {}",
                self.config.tenant, self.config.environment
            ),
        )
<<<<<<< Updated upstream
        .with_artifact("deploy/main.tf", terraform)
        .with_artifact("deploy/plan.json", plan_json);
=======
        .with_file("aws/master.tf", main_tf)
        .with_file("aws/variables.tf", variables_tf)
        .with_file("aws/plan.json", plan_json);
>>>>>>> Stashed changes

        Ok(artifacts)
    }

    async fn apply(&self, _artifacts: &ProviderArtifacts) -> Result<()> {
        info!(
            "applying AWS deployment for tenant={} env={}",
            self.config.tenant, self.config.environment
        );
        Ok(())
    }

    async fn destroy(&self, _artifacts: &ProviderArtifacts) -> Result<()> {
        info!(
            "destroying AWS deployment for tenant={} env={}",
            self.config.tenant, self.config.environment
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Action, DeployerConfig, Provider};
    use crate::plan::{DeploymentPlan, MessagingPlan, NatsPlan, TelemetryPlan};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn sample_plan() -> DeploymentPlan {
        DeploymentPlan {
            tenant: "demo".into(),
            environment: "dev".into(),
            pack_id: "demo".into(),
            pack_version: "0.1.0".into(),
            flows: Vec::new(),
            messaging: MessagingPlan {
                nats: NatsPlan {
                    cluster_name: "nats-demo".into(),
                    replicas: 1,
                    enable_jetstream: true,
                    admin_url: "https://nats.example".into(),
                },
                subjects: vec!["messaging.activities.in.demo".into()],
            },
            runners: Vec::new(),
            channels: Vec::new(),
            secrets: Vec::new(),
            oauth_clients: Vec::new(),
            telemetry: TelemetryPlan {
                otlp_endpoint: "https://otel.example".into(),
                resource_attributes: BTreeMap::new(),
            },
        }
    }

    #[tokio::test]
    async fn aws_plan_emits_artifacts() {
        let config = DeployerConfig {
            action: Action::Plan,
            provider: Provider::Aws,
            tenant: "acme".into(),
            environment: "dev".into(),
            pack_path: PathBuf::from("examples/acme-pack"),
            yes: true,
            preview: false,
        };

        let backend = AwsBackend::new(config, sample_plan());
        let artifacts = backend.plan().await.expect("plan succeeds");
        assert!(!artifacts.artifacts.is_empty());
    }
}
