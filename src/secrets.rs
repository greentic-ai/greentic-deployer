use greentic_secrets::core::{DefaultResolver, Scope, SecretUri};

use crate::config::DeployerConfig;
use crate::error::{DeployerError, Result};

pub struct SecretsAdapter {
    resolver: DefaultResolver,
    scope: Scope,
}

impl SecretsAdapter {
    pub async fn discover(config: &DeployerConfig) -> Result<Self> {
        let resolver = DefaultResolver::new()
            .await
            .map_err(|err| DeployerError::Secret(err.to_string()))?;
        let scope = Scope::new(config.environment.clone(), config.tenant.clone(), None)
            .map_err(|err| DeployerError::Secret(err.to_string()))?;
        Ok(Self { resolver, scope })
    }

    pub async fn read(&self, name: &str) -> Result<String> {
        let uri = SecretUri::new(self.scope.clone(), "configs", name)
            .map_err(|err| DeployerError::Secret(err.to_string()))?;
        self.resolver
            .get_text(&uri.to_string())
            .await
            .map_err(|err| DeployerError::Secret(err.to_string()))
    }
}
