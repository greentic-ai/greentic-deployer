use greentic_secrets::core::{DefaultResolver, ResolverConfig, Scope, SecretUri};
use greentic_types::secrets::{SecretRequirement, SecretScope};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::config::DeployerConfig;
use crate::error::{DeployerError, Result};
use crate::providers::ResolvedSecret;
use tracing::info;

pub struct SecretsContext {
    resolver: DefaultResolver,
    default_scope: SecretScope,
}

impl SecretsContext {
    pub async fn discover(config: &DeployerConfig) -> Result<Self> {
        let resolver = DefaultResolver::from_config(
            ResolverConfig::from_env()
                .tenant(config.tenant.clone())
                .dev_fallback(false),
        )
        .await
        .map_err(|err| DeployerError::Secret(err.to_string()))?;

        Ok(Self {
            resolver,
            default_scope: SecretScope {
                env: config.environment.clone(),
                tenant: config.tenant.clone(),
                team: None,
            },
        })
    }

    pub async fn fetch(&self, requirement: &SecretRequirement) -> SecretFetchOutcome {
        let scope = self.scope_for(requirement);
        let provider_path = provider_path(&scope, requirement);

        if let Some(value) = test_secret_value(&scope.env, &scope.tenant, requirement.key.as_str())
        {
            return SecretFetchOutcome::Present {
                requirement: requirement.clone(),
                provider_path,
                value,
            };
        }

        match self.fetch_from_backend(&scope, requirement).await {
            Ok(value) => SecretFetchOutcome::Present {
                requirement: requirement.clone(),
                provider_path,
                value,
            },
            Err(err) => SecretFetchOutcome::Missing {
                requirement: requirement.clone(),
                provider_path,
                error: err,
            },
        }
    }

    pub async fn push_to_provider(&self, secrets: &[ResolvedSecret]) -> Result<()> {
        if secrets.is_empty() {
            return Ok(());
        }

        for secret in secrets {
            let scope = self.scope_for(&secret.requirement);
            let uri = self.secret_uri(&scope, secret.requirement.key.as_str())?;
            self.resolver
                .put_json(&uri.to_string(), &secret.value)
                .await
                .map_err(|err| DeployerError::Secret(err.to_string()))?;
            info!(
                "pushed secret {} (format={}) to store at {}",
                secret.requirement.key.as_str(),
                secret
                    .requirement
                    .format
                    .as_ref()
                    .map(|format| format!("{format:?}"))
                    .unwrap_or_else(|| "bytes".to_string()),
                uri.to_string()
            );
        }

        Ok(())
    }

    async fn fetch_from_backend(
        &self,
        scope: &SecretScope,
        requirement: &SecretRequirement,
    ) -> Result<String> {
        let uri = self.secret_uri(scope, requirement.key.as_str())?;
        self.resolver
            .get_text(&uri.to_string())
            .await
            .map_err(|err| DeployerError::Secret(err.to_string()))
    }

    fn secret_uri(&self, scope: &SecretScope, key: &str) -> Result<SecretUri> {
        let scope = Scope::new(scope.env.clone(), scope.tenant.clone(), scope.team.clone())
            .map_err(|err| DeployerError::Secret(err.to_string()))?;
        SecretUri::new(scope, "configs", key).map_err(|err| DeployerError::Secret(err.to_string()))
    }

    fn scope_for(&self, requirement: &SecretRequirement) -> SecretScope {
        requirement
            .scope
            .clone()
            .unwrap_or_else(|| self.default_scope.clone())
    }
}

pub enum SecretFetchOutcome {
    Present {
        requirement: SecretRequirement,
        provider_path: String,
        value: String,
    },
    Missing {
        requirement: SecretRequirement,
        provider_path: String,
        error: DeployerError,
    },
}

fn provider_path(scope: &SecretScope, requirement: &SecretRequirement) -> String {
    format!(
        "secrets://{}/{}/{}/{}",
        scope.env,
        scope.tenant,
        scope.team.clone().unwrap_or_else(|| "_".to_string()),
        requirement.key.as_str()
    )
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
