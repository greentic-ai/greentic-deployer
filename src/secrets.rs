use greentic_secrets::core::{
    DefaultResolver, ResolverConfig, Scope, SecretUri, provider::Provider as SecretsProvider,
};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::config::{DeployerConfig, Provider as CliProvider};
use crate::error::{DeployerError, Result};
use crate::providers::ResolvedSecret;
use tracing::info;

pub struct SecretsContext {
    resolver: DefaultResolver,
    scope: Scope,
    tenant: String,
    environment: String,
}

impl SecretsContext {
    pub async fn discover(config: &DeployerConfig) -> Result<Self> {
        let resolver =
            DefaultResolver::from_config(ResolverConfig::from_env().tenant(config.tenant.clone()))
                .await
                .map_err(|err| DeployerError::Secret(err.to_string()))?;
        let scope = Scope::new(config.environment.clone(), config.tenant.clone(), None)
            .map_err(|err| DeployerError::Secret(err.to_string()))?;
        Ok(Self {
            resolver,
            scope,
            tenant: config.tenant.clone(),
            environment: config.environment.clone(),
        })
    }

    pub async fn resolve(&self, name: &str) -> Result<String> {
        let normalized = Self::normalize_secret_name(name);
        if let Some(value) = test_secret_value(&self.environment, &self.tenant, &normalized) {
            return Ok(value);
        }
        let uri = SecretUri::new(self.scope.clone(), "configs", normalized.clone())
            .map_err(|err| DeployerError::Secret(err.to_string()))?;
        let provider_path = self.logical_to_provider_path(name);
        self.resolver
            .get_text(&uri.to_string())
            .await
            .map_err(|err| {
                DeployerError::Secret(format!(
                    "Missing secret {name} for tenant {}, environment {}: {}. Please configure it via greentic-secrets before deploying. Target path: {provider_path}.",
                    self.tenant, self.environment, err
                ))
            })
    }

    pub fn logical_to_provider_path(&self, name: &str) -> String {
        let canonical = Self::normalize_secret_name(name);
        format!(
            "greentic/{}/{}/{}",
            self.tenant, self.environment, canonical
        )
    }

    fn normalize_secret_name(name: &str) -> String {
        name.to_ascii_lowercase()
    }

    pub async fn push_to_provider(
        &self,
        provider: CliProvider,
        secrets: &[ResolvedSecret],
    ) -> Result<()> {
        if secrets.is_empty() {
            return Ok(());
        }

        let resolver = self.provider_resolver(provider).await?;

        for secret in secrets {
            let uri = self
                .secret_uri(&secret.spec.key)
                .map_err(|err| DeployerError::Secret(err.to_string()))?;
            resolver
                .put_json(&uri.to_string(), &secret.value)
                .await
                .map_err(|err| DeployerError::Secret(err.to_string()))?;
            info!(
                "pushed secret {} to provider {} at {}",
                secret.spec.key,
                provider.as_str(),
                uri.to_string()
            );
        }

        Ok(())
    }

    fn secret_uri(&self, name: &str) -> Result<SecretUri> {
        SecretUri::new(
            self.scope.clone(),
            "configs",
            Self::normalize_secret_name(name),
        )
        .map_err(|err| DeployerError::Secret(err.to_string()))
    }

    async fn provider_resolver(&self, provider: CliProvider) -> Result<DefaultResolver> {
        let secrets_provider = match provider {
            CliProvider::Aws => SecretsProvider::Aws,
            CliProvider::Azure => SecretsProvider::Azure,
            CliProvider::Gcp => SecretsProvider::Gcp,
        };

        let config = ResolverConfig::new()
            .provider(secrets_provider)
            .tenant(self.tenant.clone())
            .dev_fallback(false);

        DefaultResolver::from_config(config)
            .await
            .map_err(|err| DeployerError::Secret(err.to_string()))
    }
}

pub fn register_test_secret(env: &str, tenant: &str, name: &str, value: &str) {
    let key = format!("{}/{}/{}", env, tenant, normalize_test_secret_name(name));
    test_secret_store()
        .lock()
        .unwrap()
        .insert(key, value.to_string());
}

pub fn clear_test_secrets() {
    test_secret_store().lock().unwrap().clear();
}

fn test_secret_store() -> &'static Mutex<HashMap<String, String>> {
    static TEST_SECRET_STORE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    TEST_SECRET_STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn normalize_test_secret_name(name: &str) -> String {
    name.to_ascii_lowercase()
}

fn test_secret_value(env: &str, tenant: &str, name: &str) -> Option<String> {
    let key = test_secret_key(env, tenant, name);
    test_secret_store().lock().unwrap().get(&key).cloned()
}

fn test_secret_key(env: &str, tenant: &str, name: &str) -> String {
    format!("{}/{}/{}", env, tenant, name)
}
