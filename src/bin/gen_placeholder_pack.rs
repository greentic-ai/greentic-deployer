use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
use indexmap::IndexMap;
use semver::Version;
use walkdir::WalkDir;
use zip::ZipWriter;
use zip::write::{ExtendedFileOptions, FileOptions};

use greentic_types::cbor::encode_pack_manifest;
use greentic_types::component::{
    ComponentCapabilities, ComponentManifest, ComponentProfiles, ResourceHints,
};
use greentic_types::flow::{Flow, FlowHasher, FlowKind, FlowMetadata};
use greentic_types::pack_manifest::{PackFlowEntry, PackKind, PackManifest, PackSignatures};
use greentic_types::{ComponentId, FlowId, PackId};

/// Generate placeholder deployment packs for each target provider.
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Comma-separated list of providers to scaffold.
    #[arg(
        long,
        value_delimiter = ',',
        default_value = "aws,azure,gcp,k8s,local,generic"
    )]
    providers: Vec<String>,

    /// Directory to emit provider pack sources.
    #[arg(long, default_value = "providers/deployer")]
    providers_dir: PathBuf,

    /// Directory to emit built .gtpack archives.
    #[arg(long, default_value = "dist")]
    dist_dir: PathBuf,

    /// Semantic version encoded in each pack.
    #[arg(long, default_value = "0.1.0")]
    pack_version: String,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let version = Version::parse(&args.pack_version).context("parse version")?;
    fs::create_dir_all(&args.providers_dir).context("create providers dir")?;
    fs::create_dir_all(&args.dist_dir).context("create dist dir")?;

    for provider in &args.providers {
        if provider.trim().is_empty() {
            bail!("provider names must be non-empty");
        }
        generate(
            &provider.trim().to_lowercase(),
            &args.providers_dir,
            &args.dist_dir,
            &version,
        )?;
    }

    Ok(())
}

fn generate(
    provider: &str,
    providers_dir: &Path,
    dist_dir: &Path,
    version: &Version,
) -> Result<()> {
    let pack_id = format!("greentic.demo.deploy.{provider}");
    let pack_dir = providers_dir.join(provider);
    if pack_dir.exists() {
        fs::remove_dir_all(&pack_dir).context("clean previous pack dir")?;
    }
    fs::create_dir_all(&pack_dir).context("create pack dir")?;

    let flow_id = format!("deploy_{provider}_iac");
    let flow_dir = pack_dir.join("flows").join(&flow_id);
    fs::create_dir_all(&flow_dir).context("create flow dir")?;

    write_flow_files(&flow_dir, &flow_id)?;
    write_component_wasm(&pack_dir, &pack_id)?;
    write_manifest(&pack_dir, provider, &pack_id, version, &flow_id)?;
    write_pack_readme(&pack_dir, provider, &pack_id)?;
    write_pack_yaml(&pack_dir, &pack_id, version, &flow_id)?;
    package_gtpack(&pack_dir, dist_dir, &pack_id)?;

    println!("generated {} (provider {})", pack_id, provider);
    Ok(())
}

fn write_flow_files(flow_dir: &Path, flow_id: &str) -> Result<()> {
    let flow = Flow {
        schema_version: "flowir-v1".into(),
        id: FlowId::try_from(flow_id).context("parse flow id")?,
        kind: FlowKind::ComponentConfig,
        entrypoints: BTreeMap::new(),
        nodes: IndexMap::with_hasher(FlowHasher::default()),
        metadata: FlowMetadata::default(),
    };

    let json = serde_json::to_string_pretty(&flow)?;
    fs::write(flow_dir.join("flow.json"), &json)?;

    let yaml = format!(
        "id: {flow_id}\nkind: component_config\nflow:\n  schema_version: flowir-v1\n  entrypoints: {{}}\n  nodes: {{}}\n  metadata: {{}}\n"
    );
    fs::write(flow_dir.join("flow.yaml"), yaml)?;
    write_flow_ygtc(flow_dir, flow_id)?;
    Ok(())
}

fn write_flow_ygtc(flow_dir: &Path, flow_id: &str) -> Result<()> {
    let content = format!(
        "id: {flow_id}\ntype: component-config\nstart: null\nparameters: {{}}\ntags: []\nschema_version: 2\nentrypoints: {{}}\nnodes: {{}}\n",
        flow_id = flow_id
    );
    fs::write(flow_dir.join("flow.ygtc"), content)?;
    Ok(())
}

fn write_component_wasm(pack_dir: &Path, pack_id: &str) -> Result<()> {
    let component_dir = pack_dir.join("components");
    fs::create_dir_all(&component_dir)?;
    let component_name = format!("{pack_id}.component.wasm");
    fs::write(component_dir.join(component_name), b"\0asm")?;
    Ok(())
}

fn write_manifest(
    pack_dir: &Path,
    provider: &str,
    pack_id: &str,
    version: &Version,
    flow_id: &str,
) -> Result<()> {
    let component_id = format!("{pack_id}.component");
    let manifest = PackManifest {
        schema_version: "pack-v1".into(),
        pack_id: PackId::try_from(pack_id).context("pack id")?,
        version: version.clone(),
        kind: PackKind::Infrastructure,
        publisher: "greentic".into(),
        components: vec![ComponentManifest {
            id: ComponentId::try_from(component_id.as_str()).context("component id")?,
            version: version.clone(),
            supports: vec![FlowKind::ComponentConfig],
            world: "greentic:test/world".into(),
            profiles: ComponentProfiles::default(),
            capabilities: ComponentCapabilities::default(),
            configurators: None,
            operations: Vec::new(),
            config_schema: None,
            resources: ResourceHints::default(),
            dev_flows: BTreeMap::new(),
        }],
        flows: vec![PackFlowEntry {
            id: FlowId::try_from(flow_id).context("flow id")?,
            kind: FlowKind::ComponentConfig,
            flow: Flow {
                schema_version: "flowir-v1".into(),
                id: FlowId::try_from(flow_id).context("flow id duplicate")?,
                kind: FlowKind::ComponentConfig,
                entrypoints: BTreeMap::new(),
                nodes: IndexMap::with_hasher(FlowHasher::default()),
                metadata: FlowMetadata::default(),
            },
            tags: Vec::new(),
            entrypoints: Vec::new(),
        }],
        dependencies: Vec::new(),
        capabilities: Vec::new(),
        secret_requirements: Vec::new(),
        signatures: PackSignatures::default(),
        bootstrap: None,
        extensions: None,
    };

    let manifest_json = serde_json::to_string_pretty(&manifest)?;
    fs::write(pack_dir.join("manifest.json"), manifest_json)?;

    let bytes = encode_pack_manifest(&manifest)?;
    fs::write(pack_dir.join("manifest.cbor"), &bytes)?;

    // Provide a minimal description for humans.
    fs::write(
        pack_dir.join("description.txt"),
        format!("Placeholder deploy pack for provider {provider}"),
    )?;

    Ok(())
}

fn write_pack_yaml(pack_dir: &Path, pack_id: &str, version: &Version, flow_id: &str) -> Result<()> {
    let content = format!(
        "schema_version: pack-v1\npack_id: {pack_id}\nversion: {version}\nkind: infrastructure\npublisher: greentic\nflows:\n  - id: {flow_id}\n    kind: component_config\n",
    );
    fs::write(pack_dir.join("pack.yaml"), content)?;
    Ok(())
}

fn write_pack_readme(pack_dir: &Path, provider: &str, pack_id: &str) -> Result<()> {
    let content = format!(
        "# {pack_id}\n\nProvider: {provider}\nThis pack writes placeholder IaC via `deploy_{provider}_iac`. Use `greentic-pack doctor` + `greentic-pack build` to validate.",
    );
    fs::write(pack_dir.join("README.md"), content)?;
    Ok(())
}

fn package_gtpack(pack_dir: &Path, dist_dir: &Path, pack_id: &str) -> Result<()> {
    let gtpack_path = dist_dir.join(format!("{pack_id}.gtpack"));
    if gtpack_path.exists() {
        fs::remove_file(&gtpack_path)?;
    }

    let file = File::create(&gtpack_path)?;
    let mut zip = ZipWriter::new(file);
    let options: FileOptions<'_, ExtendedFileOptions> =
        FileOptions::default().compression_method(zip::CompressionMethod::Stored);

    for entry in WalkDir::new(pack_dir) {
        let entry = entry?;
        let path = entry.path();
        let rel_path = path
            .strip_prefix(pack_dir)
            .map_err(|err| anyhow!("invalid relative path: {err}"))?;
        let rel_name = rel_path.display().to_string();
        if rel_name.is_empty() {
            continue;
        }
        if entry.file_type().is_dir() {
            zip.add_directory(rel_name.clone(), options.clone())?;
            continue;
        }
        let mut file = File::open(path)?;
        zip.start_file(rel_name, options.clone())?;
        io::copy(&mut file, &mut zip)?;
    }

    zip.finish()?;

    Ok(())
}
