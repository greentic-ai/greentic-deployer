use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::config::BootstrapStateBackend;
use crate::error::{DeployerError, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct BootstrapState {
    pub version: Option<String>,
    pub digest: Option<String>,
    pub installed_at: Option<u64>,
    pub environment_kind: Option<String>,
    pub last_upgrade_at: Option<u64>,
    pub rollback_ref: Option<String>,
}

impl BootstrapState {
    pub fn installed_now(version: Option<String>, digest: Option<String>) -> Self {
        Self {
            version,
            digest,
            installed_at: Some(now_ts()),
            environment_kind: None,
            last_upgrade_at: None,
            rollback_ref: None,
        }
    }

    pub fn upgraded_from(
        current: &BootstrapState,
        version: Option<String>,
        digest: Option<String>,
        rollback_ref: Option<String>,
    ) -> Self {
        Self {
            version,
            digest,
            installed_at: current.installed_at.or_else(|| Some(now_ts())),
            environment_kind: current.environment_kind.clone(),
            last_upgrade_at: Some(now_ts()),
            rollback_ref,
        }
    }
}

pub fn load_state(path: &Path) -> Result<Option<BootstrapState>> {
    if !path.exists() {
        return Ok(None);
    }
    let data = fs::read_to_string(path)?;
    let state: BootstrapState =
        serde_json::from_str(&data).map_err(|err| DeployerError::Other(err.to_string()))?;
    Ok(Some(state))
}

pub fn save_state(path: &Path, state: &BootstrapState) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json =
        serde_json::to_string_pretty(state).map_err(|err| DeployerError::Other(err.to_string()))?;
    fs::write(path, json)?;
    Ok(())
}

pub fn load_state_backend(
    backend: BootstrapStateBackend,
    file_path: &Path,
    _namespace: &str,
    _name: &str,
) -> Result<Option<BootstrapState>> {
    match backend {
        BootstrapStateBackend::File => load_state(file_path),
        BootstrapStateBackend::K8s => Err(DeployerError::Other(
            "k8s bootstrap state backend not available in this build".into(),
        )),
    }
}

pub fn save_state_backend(
    backend: BootstrapStateBackend,
    file_path: &Path,
    _namespace: &str,
    _name: &str,
    state: &BootstrapState,
) -> Result<()> {
    match backend {
        BootstrapStateBackend::File => save_state(file_path, state),
        BootstrapStateBackend::K8s => Err(DeployerError::Other(
            "k8s bootstrap state backend not available in this build".into(),
        )),
    }
}

fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn ensure_upgrade_allowed(
    state: Option<BootstrapState>,
    target_version: &Version,
) -> Result<BootstrapState> {
    let state = state.ok_or_else(|| {
        DeployerError::Other("platform not installed; run platform install first".into())
    })?;

    let current_version = state.version.as_ref().ok_or_else(|| {
        DeployerError::Other("bootstrap state missing version; reinstall required".into())
    })?;
    let current_semver = Version::parse(current_version)
        .map_err(|err| DeployerError::Other(format!("invalid version in state: {err}")))?;

    if target_version <= &current_semver {
        return Err(DeployerError::Other(format!(
            "upgrade requires a newer pack version (current {}, requested {})",
            current_semver, target_version
        )));
    }

    Ok(state)
}
