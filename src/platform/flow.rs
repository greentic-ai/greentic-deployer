use greentic_types::pack_manifest::{PackFlowEntry, PackManifest};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapResolution {
    pub install_flow: String,
    pub upgrade_flow: String,
    pub installer_component: String,
}

pub fn resolve_bootstrap(manifest: &PackManifest) -> Result<BootstrapResolution, String> {
    let bootstrap = manifest.bootstrap.as_ref();

    let install = bootstrap
        .and_then(|b| b.install_flow.clone())
        .unwrap_or_else(|| "platform_install".to_string());
    let upgrade = bootstrap
        .and_then(|b| b.upgrade_flow.clone())
        .unwrap_or_else(|| "platform_upgrade".to_string());
    let installer = bootstrap
        .and_then(|b| b.installer_component.clone())
        .unwrap_or_else(|| "installer".to_string());

    ensure_flow_exists(&install, manifest)?;
    ensure_flow_exists(&upgrade, manifest)?;

    Ok(BootstrapResolution {
        install_flow: install,
        upgrade_flow: upgrade,
        installer_component: installer,
    })
}

fn ensure_flow_exists(flow_id: &str, manifest: &PackManifest) -> Result<(), String> {
    if manifest
        .flows
        .iter()
        .any(|f: &PackFlowEntry| f.id.as_str() == flow_id)
    {
        Ok(())
    } else {
        Err(format!(
            "bootstrap flow '{}' not found in manifest (flows: {:?})",
            flow_id,
            manifest
                .flows
                .iter()
                .map(|f| f.id.as_str())
                .collect::<Vec<_>>()
        ))
    }
}
