use async_trait::async_trait;
use serde_json;
use tracing::info;

use crate::config::{DeployerConfig, Provider};
use crate::error::Result;
use crate::plan::DeploymentPlan;
use crate::providers::{ProviderArtifacts, ProviderBackend};

/// Azure-specific backend stub.
#[derive(Clone)]
pub struct AzureBackend {
    config: DeployerConfig,
    plan: DeploymentPlan,
}

impl AzureBackend {
    pub fn new(config: DeployerConfig, plan: DeploymentPlan) -> Self {
        Self { config, plan }
    }

    fn bicep_template(&self) -> String {
        let mut body = format!(
            "/* Azure Bicep for tenant {} ({}) */\nresource containerApps 'Microsoft.Web/containerApps@2023-08-01' = {{\n  name: '{}-runners'\n",
            self.config.tenant, self.config.environment, self.config.tenant
        );
        body.push_str("  properties: {\n    configuration: {\n      secrets: [\n");
        for spec in &self.plan.secrets {
            body.push_str(&format!(
                "        {{ name: '{}', value: '@sec:{}' }}\n",
                spec.name.to_ascii_lowercase(),
                spec.name
            ));
        }
        body.push_str("      ]\n      env: {\n");
        body.push_str(&format!(
            "        OTEL_EXPORTER_OTLP_ENDPOINT: '{}'\n      }}\n    }}\n  }}\n}}\n",
            self.plan.telemetry.otlp_endpoint
        ));
        body
    }
}

#[async_trait]
impl ProviderBackend for AzureBackend {
    async fn plan(&self) -> Result<ProviderArtifacts> {
        let bicep = self.bicep_template();
        let plan_json = serde_json::to_string_pretty(&self.plan)?;

        let artifacts = ProviderArtifacts::named(
            Provider::Azure,
            format!(
                "Azure deployment for tenant {} in {}",
                self.config.tenant, self.config.environment
            ),
        )
        .with_artifact("deploy/main.bicep", bicep)
        .with_artifact("deploy/plan.json", plan_json);

        Ok(artifacts)
    }

    async fn apply(&self, _artifacts: &ProviderArtifacts) -> Result<()> {
        info!(
            "applying Azure deployment for tenant={} env={}",
            self.config.tenant, self.config.environment
        );
        Ok(())
    }

    async fn destroy(&self, _artifacts: &ProviderArtifacts) -> Result<()> {
        info!(
            "destroying Azure deployment for tenant={} env={}",
            self.config.tenant, self.config.environment
        );
        Ok(())
    }
}
