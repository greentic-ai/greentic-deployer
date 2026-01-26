use std::fs;
use std::path::Path;

use crate::error::Result;

const PLACEHOLDER_PREFIX: &str = "greentic.demo.deploy.";

pub fn emit_placeholder_artifacts(
    deploy_dir: &Path,
    tenant: &str,
    environment: &str,
    pack_id: &str,
    flow_id: &str,
    plan_summary: &str,
) -> Result<bool> {
    let provider = match pack_id.strip_prefix(PLACEHOLDER_PREFIX) {
        Some(value) => value,
        None => return Ok(false),
    };

    fs::create_dir_all(deploy_dir)?;

    let readme = build_readme(
        provider,
        tenant,
        environment,
        pack_id,
        flow_id,
        plan_summary,
    );
    fs::write(deploy_dir.join("README.md"), readme)?;

    if let Some((filename, contents)) = provider_placeholder_file(provider, plan_summary) {
        fs::write(deploy_dir.join(filename), contents)?;
    }

    Ok(true)
}

fn build_readme(
    provider: &str,
    tenant: &str,
    environment: &str,
    pack_id: &str,
    flow_id: &str,
    plan_summary: &str,
) -> String {
    format!(
        "# {provider} placeholder pack\n\
Tenant: {tenant}\n\
Environment: {environment}\n\
Pack: {pack_id}\n\
Flow: {flow_id}\n\
Plan summary: {plan_summary}\n\
This pack emits placeholder IaC via the Greentic deployment flow.",
        provider = provider,
        tenant = tenant,
        environment = environment,
        pack_id = pack_id,
        flow_id = flow_id,
        plan_summary = plan_summary,
    )
}

fn provider_placeholder_file(provider: &str, plan_summary: &str) -> Option<(String, String)> {
    let summary = plan_summary.replace('\n', " ");
    match provider {
        "aws" | "azure" | "gcp" => Some((
            "main.tf".to_string(),
            format!(
                "// Placeholder IaC for {provider}\n// Summary: {summary}\n\
resource \"null_resource\" \"placeholder\" {{}}\n",
                provider = provider,
                summary = summary
            ),
        )),
        "k8s" => Some((
            "Chart.yaml".to_string(),
            format!(
                "apiVersion: v2\nname: placeholder-{provider}\nversion: 0.1.0\n\
description: Placeholder chart for {provider}\nmetadata:\n  summary: {summary}\n",
                provider = provider,
                summary = summary
            ),
        )),
        "local" => Some((
            "local.sh".to_string(),
            format!(
                "#!/usr/bin/env bash\n\
echo \"Placeholder local deployment for {provider}\"\n\
echo \"Plan summary: {summary}\"\n",
                provider = provider,
                summary = summary
            ),
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn non_placeholder_pack_skipped() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("deploy");
        let handled = emit_placeholder_artifacts(
            &target,
            "acme",
            "dev",
            "custom.pack",
            "deploy-custom",
            "summary",
        )
        .unwrap();
        assert!(!handled);
        assert!(!target.exists());
    }

    #[test]
    fn placeholder_emits_readme_and_provider_file() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("deploy");
        let handled = emit_placeholder_artifacts(
            &target,
            "acme",
            "dev",
            "greentic.demo.deploy.aws",
            "deploy_aws_iac",
            "plan summary",
        )
        .unwrap();
        assert!(handled);
        assert!(target.join("README.md").exists());
        assert!(target.join("main.tf").exists());
    }
}
