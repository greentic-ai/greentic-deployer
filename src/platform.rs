use std::path::Path;

use crate::error::{DeployerError, Result};
use crate::pack_introspect::{read_entry_from_gtpack, read_manifest_from_gtpack};
use crate::platform::flow::resolve_bootstrap;
use crate::platform::oci::compute_sha256;
use greentic_types::pack_manifest::PackManifest;
use tracing::warn;

#[derive(Debug)]
pub struct PlatformPackInfo {
    pub manifest: PackManifest,
    pub digest: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct VerificationPolicy {
    pub verify: bool,
    pub strict: bool,
}

#[derive(Debug)]
pub struct VerificationOutcome {
    pub warnings: Vec<String>,
}

pub mod flow;
pub mod oci;

pub fn load_platform_pack(path: &Path) -> Result<PlatformPackInfo> {
    if !path.exists() {
        return Err(DeployerError::Pack(format!(
            "pack {} does not exist",
            path.display()
        )));
    }
    if !path.is_file() {
        return Err(DeployerError::Pack(format!(
            "pack {} is not a file",
            path.display()
        )));
    }

    let digest = compute_sha256(path)?;
    let manifest = read_manifest_from_gtpack(path)?;

    Ok(PlatformPackInfo { manifest, digest })
}

pub fn load_bootstrap_flow(path: &Path, manifest: &PackManifest, install: bool) -> Result<Vec<u8>> {
    let bootstrap = resolve_bootstrap(manifest)
        .map_err(|err| DeployerError::Pack(format!("bootstrap resolution error: {err}")))?;
    let flow_id = if install {
        bootstrap.install_flow
    } else {
        bootstrap.upgrade_flow
    };
    let entry = Path::new("flows").join(format!("{}.ygtc", flow_id));
    read_entry_from_gtpack(path, &entry)
}

pub fn verify_platform_pack(
    info: &PlatformPackInfo,
    policy: VerificationPolicy,
) -> Result<VerificationOutcome> {
    let mut warnings = Vec::new();
    if !policy.verify {
        warnings.push("verification skipped (--verify=false)".to_string());
        return Ok(VerificationOutcome { warnings });
    }

    match info.manifest.signatures.signatures.as_slice() {
        [] => {
            let msg = "pack missing signatures";
            if policy.strict {
                return Err(DeployerError::Pack(format!(
                    "{msg}; pass --verify=false to bypass"
                )));
            } else {
                warnings.push(msg.to_string());
            }
        }
        sigs => {
            for sig in sigs {
                if sig.signature.is_empty() {
                    return Err(DeployerError::Pack(format!(
                        "invalid signature for key_id={} (empty payload)",
                        sig.key_id
                    )));
                }
            }
        }
    }

    if !warnings.is_empty() {
        for w in &warnings {
            warn!("{w}");
        }
    }

    Ok(VerificationOutcome { warnings })
}
