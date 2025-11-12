use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_cbor;
use serde_json;
use zip::ZipArchive;

use crate::config::DeployerConfig;
use crate::error::{DeployerError, Result};
use crate::plan::DeploymentPlan;
use greentic_flow::FlowBundle;
use greentic_flow::load_and_validate_bundle;
use greentic_pack::builder::PackManifest;

/// Build a cloud-agnostic deployment plan from a Greentic pack.
pub fn build_plan(config: &DeployerConfig) -> Result<DeploymentPlan> {
    let mut source = PackSource::open(&config.pack_path)?;
    let manifest = source.read_manifest()?;
    let flows = source.load_flows(&manifest)?;
    Ok(DeploymentPlan::from_manifest(config, &manifest, &flows))
}

struct PackSource {
    inner: PackSourceInner,
}

enum PackSourceInner {
    Archive(ZipArchive<File>),
    Directory(PathBuf),
}

impl PackSource {
    fn open(path: &Path) -> Result<Self> {
        if path.is_dir() {
            Ok(Self {
                inner: PackSourceInner::Directory(path.to_path_buf()),
            })
        } else {
            let file = File::open(path)?;
            let archive = ZipArchive::new(file)?;
            Ok(Self {
                inner: PackSourceInner::Archive(archive),
            })
        }
    }

    fn read_manifest(&mut self) -> Result<PackManifest> {
        match &mut self.inner {
            PackSourceInner::Archive(archive) => {
                let mut manifest = Vec::new();
                let mut entry = archive.by_name("manifest.cbor")?;
                entry.read_to_end(&mut manifest)?;
                Ok(serde_cbor::from_slice(&manifest)?)
            }
            PackSourceInner::Directory(dir) => read_manifest_from_directory(dir),
        }
    }

    fn load_flows(&mut self, manifest: &PackManifest) -> Result<Vec<FlowBundle>> {
        manifest
            .flows
            .iter()
            .map(|entry| self.load_flow(entry))
            .collect()
    }

    fn load_flow(&mut self, entry: &greentic_pack::builder::FlowEntry) -> Result<FlowBundle> {
        let yaml = self.read_flow_yaml(&entry.file_yaml)?;
        load_and_validate_bundle(&yaml, None).map_err(|err| DeployerError::Pack(err.to_string()))
    }

    fn read_flow_yaml(&mut self, relative_path: &str) -> Result<String> {
        match &mut self.inner {
            PackSourceInner::Archive(archive) => {
                let mut entry = archive.by_name(relative_path)?;
                let mut contents = String::new();
                entry.read_to_string(&mut contents)?;
                Ok(contents)
            }
            PackSourceInner::Directory(root) => {
                let path = root.join(relative_path);
                let contents = fs::read_to_string(path)?;
                Ok(contents)
            }
        }
    }
}

fn read_manifest_from_directory(root: &Path) -> Result<PackManifest> {
    let cbor = root.join("manifest.cbor");
    let json = root.join("manifest.json");

    if cbor.exists() {
        let bytes = fs::read(cbor)?;
        Ok(serde_cbor::from_slice(&bytes)?)
    } else if json.exists() {
        let bytes = fs::read(json)?;
        Ok(serde_json::from_slice(&bytes)?)
    } else {
        Err(DeployerError::Pack(format!(
            "pack manifest missing in {}",
            root.display()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Action, DeployerConfig, Provider};
    use std::path::PathBuf;

    #[test]
    fn builds_plan_from_example_pack() {
        let config = DeployerConfig {
            action: Action::Plan,
            provider: Provider::Aws,
            tenant: "acme".into(),
            environment: "staging".into(),
            pack_path: PathBuf::from("examples/acme-pack"),
            yes: true,
            preview: false,
        };

        let plan = build_plan(&config).expect("should build plan");
        assert_eq!(plan.flows.len(), 1);
        assert!(plan.channels.iter().any(|c| c.channel_type == "messaging"));
        assert!(plan.secrets.iter().any(|s| s.name == "SLACK_BOT_TOKEN"));
    }
}
