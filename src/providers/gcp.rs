use async_trait::async_trait;
use serde_json;
use tracing::info;

use crate::config::{DeployerConfig, Provider};
use crate::error::Result;
use crate::plan::DeploymentPlan;
use crate::providers::{ProviderArtifacts, ProviderBackend};

/// GCP-specific backend stub.
#[derive(Clone)]
pub struct GcpBackend {
    config: DeployerConfig,
    plan: DeploymentPlan,
}

impl GcpBackend {
    pub fn new(config: DeployerConfig, plan: DeploymentPlan) -> Self {
        Self { config, plan }
    }

    fn config_yaml(&self) -> String {
        let mut docs = format!(
            "resources:\n  - name: {}-runner\n    type: run.v1.service\n    properties:\n      template:\n        spec:\n          containers:\n          - image: gcr.io/greentic/runner:latest\n            env:\n",
            self.config.tenant
        );
        for secret in &self.plan.secrets {
            docs.push_str(&format!(
                "            - name: {}\n              value: projects/runner/secrets/{}/versions/latest\n",
                secret.name,
                secret.name.to_ascii_lowercase()
            ));
        }
        docs.push_str("          - name: OTEL_EXPORTER_OTLP_ENDPOINT\n            value: \"");
        docs.push_str(&self.plan.telemetry.otlp_endpoint);
        docs.push_str("\"\n");
        docs.push_str("      metadata:\n");
        docs.push_str(&format!(
            "        annotations:\n          greentic-tenant: {}\n          greentic-environment: {}\n",
            self.config.tenant, self.config.environment
        ));
        docs
    }
}

#[async_trait]
impl ProviderBackend for GcpBackend {
    async fn plan(&self) -> Result<ProviderArtifacts> {
        let yaml = self.config_yaml();
        let plan_json = serde_json::to_string_pretty(&self.plan)?;

        let artifacts = ProviderArtifacts::named(
            Provider::Gcp,
            format!(
                "GCP deployment for tenant {} in {}",
                self.config.tenant, self.config.environment
            ),
        )
<<<<<<< Updated upstream
        .with_artifact("deploy/main.yaml", yaml)
        .with_artifact("deploy/plan.json", plan_json);
=======
        .with_file("gcp/master.yaml", yaml)
        .with_file("gcp/parameters.yaml", parameters)
        .with_file("gcp/plan.json", plan_json);
>>>>>>> Stashed changes

        Ok(artifacts)
    }

    async fn apply(&self, _artifacts: &ProviderArtifacts) -> Result<()> {
        info!(
            "applying GCP deployment for tenant={} env={}",
            self.config.tenant, self.config.environment
        );
        Ok(())
    }

    async fn destroy(&self, _artifacts: &ProviderArtifacts) -> Result<()> {
        info!(
            "destroying GCP deployment for tenant={} env={}",
            self.config.tenant, self.config.environment
        );
        Ok(())
    }
}
