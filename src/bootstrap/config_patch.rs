use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::{DeployerError, Result};

#[derive(Debug, Clone)]
pub struct ConfigSnapshot {
    pub existed: bool,
    pub content: Option<String>,
}

/// Default path for storing the applied config patch (sibling to bootstrap state).
pub fn default_config_patch_path(state_path: &Path) -> PathBuf {
    match state_path.parent() {
        Some(dir) if dir != Path::new("") => dir.join("config_patch.json"),
        _ => PathBuf::from("config_patch.json"),
    }
}

pub fn snapshot_config(path: &Path) -> Result<ConfigSnapshot> {
    if path.exists() {
        let content = fs::read_to_string(path)?;
        Ok(ConfigSnapshot {
            existed: true,
            content: Some(content),
        })
    } else {
        Ok(ConfigSnapshot {
            existed: false,
            content: None,
        })
    }
}

/// Apply a config patch by writing it to disk as JSON (no merge yet).
pub fn apply_config_patch(path: &Path, patch: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let rendered =
        serde_json::to_string_pretty(patch).map_err(|err| DeployerError::Other(err.to_string()))?;
    fs::write(path, rendered)?;
    Ok(())
}

pub fn restore_config(path: &Path, snapshot: &ConfigSnapshot) -> Result<()> {
    if snapshot.existed {
        let content = snapshot
            .content
            .as_ref()
            .cloned()
            .unwrap_or_else(|| "".to_string());
        fs::write(path, content)?;
    } else if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}
