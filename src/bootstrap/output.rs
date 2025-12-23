use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SecretWrite {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BootstrapOutput {
    pub output_version: String,
    #[serde(default)]
    pub config_patch: Value,
    #[serde(default)]
    pub secrets_writes: Vec<SecretWrite>,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub ready: bool,
}

impl BootstrapOutput {
    pub fn new(
        config_patch: Value,
        secrets_writes: Vec<SecretWrite>,
        warnings: Vec<String>,
        ready: bool,
    ) -> Self {
        Self {
            output_version: "v1".to_string(),
            config_patch,
            secrets_writes,
            warnings,
            ready,
        }
    }

    pub fn redacted(&self) -> Self {
        let mut redacted = self.clone();
        redacted.secrets_writes = redacted
            .secrets_writes
            .iter()
            .map(|s| SecretWrite {
                key: s.key.clone(),
                value: None,
                scope: s.scope.clone(),
                metadata: s.metadata.clone(),
            })
            .collect();
        redacted
    }
}
