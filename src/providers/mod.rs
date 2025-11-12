use std::path::PathBuf;

use async_trait::async_trait;

use crate::config::Provider;
use crate::error::Result;
use crate::plan::DeploymentPlan;

pub mod aws;
pub mod azure;
pub mod gcp;

pub use aws::AwsBackend;
pub use azure::AzureBackend;
pub use gcp::GcpBackend;

/// Files produced by provider backends.
#[derive(Debug, Clone)]
pub struct ProviderArtifact {
    pub path: PathBuf,
    pub contents: String,
}

/// Output bundle generated during `plan`.
#[derive(Debug, Clone)]
pub struct ProviderArtifacts {
    pub provider: Provider,
    pub description: String,
    pub artifacts: Vec<ProviderArtifact>,
}

impl ProviderArtifacts {
    pub fn named(provider: Provider, description: String) -> Self {
        Self {
            provider,
            description,
            artifacts: Vec::new(),
        }
    }

    pub fn with_artifact(mut self, path: impl Into<PathBuf>, contents: impl Into<String>) -> Self {
        self.artifacts.push(ProviderArtifact {
            path: path.into(),
            contents: contents.into(),
        });
        self
    }
}

/// Provider backend interface.
#[async_trait]
pub trait ProviderBackend: Send + Sync {
    async fn plan(&self) -> Result<ProviderArtifacts>;
    async fn apply(&self, artifacts: &ProviderArtifacts) -> Result<()>;
    async fn destroy(&self, artifacts: &ProviderArtifacts) -> Result<()>;
}

/// Create a provider backend instance for the requested provider.
pub fn create_backend(
    provider: Provider,
    config: &crate::config::DeployerConfig,
    plan: &DeploymentPlan,
) -> Result<Box<dyn ProviderBackend>> {
    match provider {
        Provider::Aws => Ok(Box::new(AwsBackend::new(config.clone(), plan.clone()))),
        Provider::Azure => Ok(Box::new(AzureBackend::new(config.clone(), plan.clone()))),
        Provider::Gcp => Ok(Box::new(GcpBackend::new(config.clone(), plan.clone()))),
    }
}
