use std::collections::BTreeMap;
use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::config::{DeployerConfig, Provider};
use crate::error::Result;
use crate::plan::{PlanContext, SecretContext};

pub mod aws;
pub mod azure;
pub mod gcp;

pub use aws::AwsBackend;
pub use azure::AzureBackend;
pub use gcp::GcpBackend;

/// Contract for a generated artifact file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedFile {
    pub relative_path: PathBuf,
    pub contents: String,
}

/// Resolved secret metadata emitted during apply/destroy.
#[derive(Debug, Clone)]
pub struct ResolvedSecret {
    pub spec: SecretContext,
    pub value: String,
    pub provider_path: String,
}

impl ResolvedSecret {
    pub fn value_len(&self) -> usize {
        self.value.len()
    }
}

/// Output bundle emitted by a provider backend during `plan`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderArtifacts {
    pub provider: Provider,
    pub description: String,
    pub plan: PlanContext,
    pub files: Vec<GeneratedFile>,
}

impl ProviderArtifacts {
    pub fn new(provider: Provider, description: String, plan: PlanContext) -> Self {
        Self {
            provider,
            description,
            plan,
            files: Vec::new(),
        }
    }

    pub fn named(provider: Provider, description: String, plan: PlanContext) -> Self {
        Self::new(provider, description, plan)
    }

    pub fn with_file(mut self, path: impl Into<PathBuf>, contents: impl Into<String>) -> Self {
        self.files.push(GeneratedFile {
            relative_path: path.into(),
            contents: contents.into(),
        });
        self
    }
}

/// Manifest describing required secrets, OAuth clients, telemetry, and artifacts.
#[derive(Debug, Clone, Serialize)]
pub struct ApplyManifest {
    pub action: String,
    pub provider: Provider,
    pub tenant: String,
    pub environment: String,
    pub pack_id: String,
    pub pack_version: String,
    pub secrets: Vec<ApplySecret>,
    pub oauth_clients: Vec<ApplyOAuthClient>,
    pub telemetry: TelemetryManifest,
    pub artifacts: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplySecret {
    pub logical_name: String,
    pub provider_path: String,
    pub value_length: usize,
    pub scope: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplyOAuthClient {
    pub provider: String,
    pub scopes: Vec<String>,
    pub redirect_urls: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TelemetryManifest {
    pub endpoint: String,
    pub resource_attributes: BTreeMap<String, String>,
}

impl ApplyManifest {
    pub fn build(
        action: &str,
        config: &DeployerConfig,
        artifacts: &ProviderArtifacts,
        secrets: &[ResolvedSecret],
    ) -> Self {
        let plan = &artifacts.plan;
        let secret_entries = secrets
            .iter()
            .map(|entry| ApplySecret {
                logical_name: entry.spec.key.clone(),
                provider_path: entry.provider_path.clone(),
                value_length: entry.value_len(),
                scope: entry.spec.scope.clone(),
            })
            .collect();

        let oauth_clients = plan
            .plan
            .oauth
            .iter()
            .map(|client| ApplyOAuthClient {
                provider: client.provider_id.clone(),
                scopes: Vec::new(),
                redirect_urls: vec![client.redirect_path.clone()],
            })
            .collect();

        let telemetry = TelemetryManifest {
            endpoint: plan.telemetry.otlp_endpoint.clone(),
            resource_attributes: plan.telemetry.resource_attributes.clone(),
        };

        let artifacts_list = artifacts
            .files
            .iter()
            .map(|file| file.relative_path.display().to_string())
            .collect();

        Self {
            action: action.to_string(),
            provider: config.provider,
            tenant: config.tenant.clone(),
            environment: config.environment.clone(),
            pack_id: plan.plan.pack_id.clone(),
            pack_version: plan.plan.pack_version.to_string(),
            secrets: secret_entries,
            oauth_clients,
            telemetry,
            artifacts: artifacts_list,
        }
    }
}

/// Provider backend interface.
#[async_trait]
pub trait ProviderBackend: Send + Sync {
    async fn plan(&self) -> Result<ProviderArtifacts>;
    async fn apply(&self, artifacts: &ProviderArtifacts, secrets: &[ResolvedSecret]) -> Result<()>;
    async fn destroy(
        &self,
        artifacts: &ProviderArtifacts,
        secrets: &[ResolvedSecret],
    ) -> Result<()>;
}

/// Create a provider backend instance for the requested provider.
pub fn create_backend(
    provider: Provider,
    config: &crate::config::DeployerConfig,
    plan: &PlanContext,
) -> Result<Box<dyn ProviderBackend>> {
    match provider {
        Provider::Aws => Ok(Box::new(AwsBackend::new(config.clone(), plan.clone()))),
        Provider::Azure => Ok(Box::new(AzureBackend::new(config.clone(), plan.clone()))),
        Provider::Gcp => Ok(Box::new(GcpBackend::new(config.clone(), plan.clone()))),
    }
}
