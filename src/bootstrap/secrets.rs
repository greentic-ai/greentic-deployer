use std::fs;
use std::path::{Path, PathBuf};

use base64::{Engine as _, engine::general_purpose};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::env;
use std::sync::Mutex;
use std::sync::OnceLock;

use crate::bootstrap::output::SecretWrite;
use crate::error::{DeployerError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretsBackend {
    File(PathBuf),
    K8s { namespace: String, name: String },
}

static K8S_SECRET_DIR_OVERRIDE: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

pub fn set_k8s_secret_dir_override(path: Option<PathBuf>) {
    let lock = K8S_SECRET_DIR_OVERRIDE.get_or_init(|| Mutex::new(None));
    *lock.lock().unwrap() = path;
}

#[derive(Debug, Clone)]
pub struct SecretsSnapshot {
    pub backend: SecretsBackend,
    pub content: Option<String>,
}

pub fn parse_backend(input: &str) -> Result<SecretsBackend> {
    if let Some(path) = input.strip_prefix("file:") {
        if path.is_empty() {
            return Err(DeployerError::Config(
                "file secrets backend requires a path".into(),
            ));
        }
        return Ok(SecretsBackend::File(PathBuf::from(path)));
    }

    if let Some(rest) = input.strip_prefix("k8s:") {
        if rest.contains('/') {
            let mut parts = rest.splitn(2, '/');
            let ns = parts
                .next()
                .ok_or_else(|| DeployerError::Config("k8s backend missing namespace".into()))?;
            let name = parts
                .next()
                .ok_or_else(|| DeployerError::Config("k8s backend missing secret name".into()))?;
            return Ok(SecretsBackend::K8s {
                namespace: ns.to_string(),
                name: name.to_string(),
            });
        } else if rest.contains('=') {
            // format: k8s:namespace=ns,name=secret
            let mut ns: Option<String> = None;
            let mut name: Option<String> = None;
            for part in rest.split(',') {
                let mut kv = part.splitn(2, '=');
                let key = kv.next();
                let val = kv.next();
                match (key, val) {
                    (Some("namespace"), Some(v)) => ns = Some(v.to_string()),
                    (Some("name"), Some(v)) => name = Some(v.to_string()),
                    _ => continue,
                }
            }
            let ns = ns.ok_or_else(|| {
                DeployerError::Config("k8s backend requires namespace=<ns>".into())
            })?;
            let name = name.ok_or_else(|| {
                DeployerError::Config("k8s backend requires name=<secret>".into())
            })?;
            return Ok(SecretsBackend::K8s {
                namespace: ns,
                name,
            });
        } else {
            return Err(DeployerError::Config(
                "k8s backend expects k8s:<namespace>/<name> or k8s:namespace=...".into(),
            ));
        }
    }

    Err(DeployerError::Config(format!(
        "unsupported secrets backend: {input}"
    )))
}

pub fn snapshot_backend(backend: &SecretsBackend) -> Option<SecretsSnapshot> {
    match backend {
        SecretsBackend::File(path) => {
            if path.exists() {
                match fs::read_to_string(path) {
                    Ok(content) => Some(SecretsSnapshot {
                        backend: backend.clone(),
                        content: Some(content),
                    }),
                    Err(_) => None,
                }
            } else {
                Some(SecretsSnapshot {
                    backend: backend.clone(),
                    content: None,
                })
            }
        }
        SecretsBackend::K8s { .. } => None,
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredSecret {
    value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    metadata: Option<Value>,
}

pub fn execute_writes(backend: &SecretsBackend, writes: &[SecretWrite]) -> Result<()> {
    if writes.is_empty() {
        return Ok(());
    }
    match backend {
        SecretsBackend::File(path) => write_file_backend(path, writes),
        SecretsBackend::K8s { namespace, name } => write_k8s_backend(namespace, name, writes),
    }
}

fn write_file_backend(path: &Path, writes: &[SecretWrite]) -> Result<()> {
    let mut store: Map<String, Value> = if path.exists() {
        let content = fs::read_to_string(path)?;
        if content.trim().is_empty() {
            Map::new()
        } else {
            serde_json::from_str(&content)?
        }
    } else {
        Map::new()
    };

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    for write in writes {
        let value = write.value.clone().ok_or_else(|| {
            DeployerError::Config(format!(
                "secret write '{}' missing value (installers must supply values in bootstrap mode)",
                write.key
            ))
        })?;
        let storage_key = storage_key(write);
        let record = StoredSecret {
            value,
            scope: write.scope.clone(),
            metadata: write.metadata.clone(),
        };
        store.insert(storage_key, serde_json::to_value(record)?);
    }

    let serialized = serde_json::to_string_pretty(&store)?;
    fs::write(path, serialized)?;
    Ok(())
}

pub fn restore_backend(snapshot: &SecretsSnapshot) -> Result<()> {
    match &snapshot.backend {
        SecretsBackend::File(path) => {
            if let Some(content) = &snapshot.content {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(path, content)?;
            } else if path.exists() {
                fs::remove_file(path)?;
            }
            Ok(())
        }
        SecretsBackend::K8s { .. } => Err(DeployerError::Other(
            "k8s secrets backend restore not implemented in stub".into(),
        )),
    }
}

fn storage_key(write: &SecretWrite) -> String {
    match write.scope.as_ref() {
        Some(scope) => format!("{}/{}", scope, write.key),
        None => write.key.clone(),
    }
}

fn write_k8s_backend(namespace: &str, name: &str, writes: &[SecretWrite]) -> Result<()> {
    let override_dir = K8S_SECRET_DIR_OVERRIDE
        .get()
        .and_then(|lock| lock.lock().ok().and_then(|v| v.clone()));
    let base_dir = override_dir
        .map(|p| p.to_string_lossy().to_string())
        .or_else(|| env::var("GREENTIC_K8S_SECRET_DIR").ok())
        .unwrap_or_else(|| "/tmp/greentic-k8s-secrets".to_string());
    let secret_dir = Path::new(&base_dir).join(namespace);
    fs::create_dir_all(&secret_dir)?;
    let path = secret_dir.join(format!("{name}.yaml"));

    let mut data = serde_json::Map::new();
    for write in writes {
        let value = write.value.clone().ok_or_else(|| {
            DeployerError::Config(format!(
                "secret write '{}' missing value (installers must supply values in bootstrap mode)",
                write.key
            ))
        })?;
        let key = storage_key(write);
        let encoded = general_purpose::STANDARD.encode(value.as_bytes());
        data.insert(key, Value::String(encoded));
    }

    // Build a minimal Secret manifest
    let manifest = json!({
        "apiVersion": "v1",
        "kind": "Secret",
        "metadata": {
            "name": name,
            "namespace": namespace,
            "annotations": {
                "managed-by": "greentic-deployer"
            }
        },
        "type": "Opaque",
        "data": data
    });

    let yaml =
        serde_yaml_bw::to_string(&manifest).map_err(|err| DeployerError::Other(err.to_string()))?;
    fs::write(path, yaml)?;
    Ok(())
}
