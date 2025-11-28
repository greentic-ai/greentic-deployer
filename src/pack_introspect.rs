use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use greentic_pack::builder::PackManifest;
use greentic_pack::plan::infer_base_deployment_plan;
use greentic_types::component::ComponentManifest;
use greentic_types::deployment::{DeploymentPlan, OAuthPlan, SecretPlan};
use greentic_types::{EnvId, TenantCtx, TenantId};
use serde_cbor;
use serde_json::{self, Map as JsonMap, Value as JsonValue};
use zip::ZipArchive;

use crate::config::DeployerConfig;
use crate::error::{DeployerError, Result};
use crate::plan::{
    ComponentRole, DeploymentHints, DeploymentProfile, InferenceNotes, InfraPlan, PlanContext,
    PlannedComponent, Target, assemble_plan,
};

/// Build a plan context from the provided pack.
pub fn build_plan(config: &DeployerConfig) -> Result<PlanContext> {
    let mut source = PackSource::open(&config.pack_path)?;
    let manifest = source.read_manifest()?;
    let components = source.load_component_manifests(&manifest)?;
    let tenant_ctx = build_tenant_ctx(config)?;
    let connectors = manifest.meta.annotations.get("connectors");
    let mut base = infer_base_deployment_plan(
        &manifest.meta,
        &manifest.flows,
        connectors,
        &components,
        &tenant_ctx,
        &config.environment,
    );
    merge_annotation_secrets(&mut base, &manifest.meta.annotations);
    merge_annotation_oauth(&mut base, &manifest.meta.annotations, config);
    let deployment = build_deployment_hints(&manifest.meta.annotations, config);
    let components = infer_component_profiles(
        &manifest,
        &components,
        &deployment,
        &manifest.meta.annotations,
    );
    Ok(assemble_plan(base, config, deployment, components))
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

    fn load_component_manifests(
        &mut self,
        manifest: &PackManifest,
    ) -> Result<HashMap<String, ComponentManifest>> {
        let mut components = HashMap::new();
        for entry in &manifest.components {
            if let Some(path) = &entry.manifest_file {
                let json = self.read_file_to_string(path)?;
                let parsed: ComponentManifest = serde_json::from_str(&json).map_err(|err| {
                    DeployerError::Pack(format!("component manifest {} is invalid: {}", path, err))
                })?;
                components.insert(parsed.id.to_string(), parsed);
            }
        }
        Ok(components)
    }

    fn read_file_to_string(&mut self, relative_path: &str) -> Result<String> {
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

fn build_tenant_ctx(config: &DeployerConfig) -> Result<TenantCtx> {
    let env_id = EnvId::from_str(&config.environment).map_err(|err| {
        DeployerError::Config(format!(
            "invalid environment '{}': {}",
            config.environment, err
        ))
    })?;
    let tenant_id = TenantId::from_str(&config.tenant).map_err(|err| {
        DeployerError::Config(format!("invalid tenant '{}': {}", config.tenant, err))
    })?;
    Ok(TenantCtx::new(env_id, tenant_id))
}

fn merge_annotation_secrets(plan: &mut DeploymentPlan, annotations: &JsonMap<String, JsonValue>) {
    let Some(secret_map) = annotations
        .get("greentic.secrets")
        .and_then(|value| value.as_object())
    else {
        return;
    };

    for key in secret_map.keys() {
        if plan.secrets.iter().any(|entry| entry.key == *key) {
            continue;
        }
        plan.secrets.push(SecretPlan {
            key: key.clone(),
            required: true,
            scope: "tenant".to_string(),
        });
    }
}

fn merge_annotation_oauth(
    plan: &mut DeploymentPlan,
    annotations: &JsonMap<String, JsonValue>,
    config: &DeployerConfig,
) {
    let Some(oauth_map) = annotations
        .get("greentic.oauth")
        .and_then(|value| value.as_object())
    else {
        return;
    };

    if oauth_map.is_empty() {
        return;
    }

    plan.oauth = oauth_map
        .iter()
        .map(|(provider, entry)| {
            let logical_client_id = entry
                .get("client_id")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string())
                .unwrap_or_else(|| format!("{}-{}-{provider}", config.tenant, config.environment));

            let redirect_path = format!(
                "/oauth/{provider}/callback/{tenant}/{environment}",
                tenant = config.tenant,
                environment = config.environment
            );

            OAuthPlan {
                provider_id: provider.clone(),
                logical_client_id,
                redirect_path,
                extra: entry.clone(),
            }
        })
        .collect();
}

fn build_deployment_hints(
    annotations: &JsonMap<String, JsonValue>,
    config: &DeployerConfig,
) -> DeploymentHints {
    let mut provider = None;
    let mut strategy = None;
    if let Some(value) = annotations.get("greentic.deployment") {
        match value {
            JsonValue::String(s) => strategy = Some(s.to_string()),
            JsonValue::Object(map) => {
                if let Some(val) = map.get("provider").and_then(|v| v.as_str()) {
                    provider = Some(val.to_string());
                }
                if let Some(val) = map.get("strategy").and_then(|v| v.as_str()) {
                    strategy = Some(val.to_string());
                }
            }
            _ => {}
        }
    }

    let provider_name = provider.unwrap_or_else(|| config.provider.as_str().to_string());
    let target =
        target_from_provider(&provider_name).unwrap_or_else(|| Target::from(config.provider));

    DeploymentHints {
        target,
        provider: provider_name,
        strategy: strategy.unwrap_or_else(|| config.strategy.clone()),
    }
}

fn target_from_provider(provider: &str) -> Option<Target> {
    match provider.to_ascii_lowercase().as_str() {
        "local" | "dev" => Some(Target::Local),
        "aws" => Some(Target::Aws),
        "azure" => Some(Target::Azure),
        "gcp" => Some(Target::Gcp),
        "k8s" | "kubernetes" => Some(Target::K8s),
        _ => None,
    }
}

#[derive(Debug, Clone)]
struct ComponentInfo {
    world: Option<String>,
    tags: Vec<String>,
}

fn infer_component_profiles(
    manifest: &PackManifest,
    manifests: &HashMap<String, ComponentManifest>,
    deployment: &DeploymentHints,
    annotations: &JsonMap<String, JsonValue>,
) -> Vec<PlannedComponent> {
    let info_map = collect_component_info(manifest, manifests, annotations);
    let mut roles = infer_component_roles(manifest);
    let mut ids: HashSet<String> = info_map.keys().cloned().collect();
    ids.extend(roles.keys().cloned());

    // Assign a safe default role for components without an explicit mapping.
    for id in &ids {
        roles.entry(id.clone()).or_insert(ComponentRole::Worker);
    }

    let mut planned = Vec::new();
    for id in ids {
        let info = info_map.get(&id);
        let explicit_profile = declared_profile(annotations, &id);
        let tags = info
            .map(|entry| entry.tags.clone())
            .unwrap_or_else(|| tags_for_component(annotations, &id));
        let world = info.and_then(|entry| entry.world.as_deref());
        let role = roles.get(&id).cloned().unwrap_or(ComponentRole::Worker);
        let (profile, inference) = infer_profile(&id, explicit_profile, &role, world, &tags);
        let infra = map_profile_to_infra(&deployment.target, &profile);
        planned.push(PlannedComponent {
            id,
            role,
            profile,
            target: deployment.target.clone(),
            infra,
            inference,
        });
    }

    planned.sort_by(|a, b| a.id.cmp(&b.id));
    planned
}

fn collect_component_info(
    manifest: &PackManifest,
    manifests: &HashMap<String, ComponentManifest>,
    annotations: &JsonMap<String, JsonValue>,
) -> HashMap<String, ComponentInfo> {
    let mut map = HashMap::new();
    for entry in &manifest.components {
        map.entry(entry.name.clone())
            .or_insert_with(|| ComponentInfo {
                world: entry.world.clone(),
                tags: tags_for_component(annotations, &entry.name),
            });
    }

    for (id, component) in manifests {
        map.entry(id.clone())
            .and_modify(|info| info.world = Some(component.world.clone()))
            .or_insert_with(|| ComponentInfo {
                world: Some(component.world.clone()),
                tags: tags_for_component(annotations, id),
            });
    }

    map
}

fn infer_component_roles(manifest: &PackManifest) -> HashMap<String, ComponentRole> {
    let mut roles = HashMap::new();

    if let Some(events) = &manifest.meta.events {
        for provider in &events.providers {
            let role = match provider.kind {
                greentic_pack::events::EventProviderKind::Bridge => ComponentRole::EventBridge,
                _ => ComponentRole::EventProvider,
            };
            roles.insert(provider.component.clone(), role);
        }
    }

    if let Some(messaging) = manifest
        .meta
        .messaging
        .as_ref()
        .and_then(|entry| entry.adapters.as_ref())
    {
        for adapter in messaging {
            roles.insert(adapter.component.clone(), ComponentRole::MessagingAdapter);
        }
    }

    roles
}

fn declared_profile(
    annotations: &JsonMap<String, JsonValue>,
    component_id: &str,
) -> Option<DeploymentProfile> {
    let deployment = annotations
        .get("greentic.deployment")
        .and_then(|value| value.as_object());

    let profile_value = deployment
        .and_then(|map| map.get("profiles"))
        .and_then(|value| value.as_object())
        .and_then(|profiles| profiles.get(component_id))
        .or_else(|| {
            annotations
                .get("greentic.deployment.profile")
                .or_else(|| deployment.and_then(|map| map.get("profile")))
        });

    profile_value
        .and_then(|value| value.as_str())
        .and_then(parse_profile)
}

fn infer_profile(
    component_id: &str,
    explicit: Option<DeploymentProfile>,
    role: &ComponentRole,
    world: Option<&str>,
    tags: &[String],
) -> (DeploymentProfile, Option<InferenceNotes>) {
    if let Some(profile) = explicit {
        return (
            profile,
            Some(InferenceNotes {
                source: "explicit profile from pack metadata".to_string(),
                warnings: Vec::new(),
            }),
        );
    }

    if let Some((profile, source)) = profile_from_tags(tags) {
        return (
            profile,
            Some(InferenceNotes {
                source,
                warnings: Vec::new(),
            }),
        );
    }

    if let Some(world) = world
        && let Some((profile, source)) = profile_from_world(world)
    {
        return (
            profile,
            Some(InferenceNotes {
                source,
                warnings: Vec::new(),
            }),
        );
    }

    let (profile, warning) = default_profile(role);
    let warnings = if warning {
        vec![format!(
            "component {component_id} (role={}) has no deployment profile hints; defaulting to {:?}",
            role_label(role),
            profile
        )]
    } else {
        Vec::new()
    };

    (
        profile,
        Some(InferenceNotes {
            source: if warning {
                "defaulted profile due to missing hints".to_string()
            } else {
                "role-based default profile".to_string()
            },
            warnings,
        }),
    )
}

fn role_label(role: &ComponentRole) -> &'static str {
    match role {
        ComponentRole::EventProvider => "event-provider",
        ComponentRole::EventBridge => "event-bridge",
        ComponentRole::MessagingAdapter => "messaging-adapter",
        ComponentRole::Worker => "worker",
        ComponentRole::Other => "component",
    }
}

fn profile_from_tags(tags: &[String]) -> Option<(DeploymentProfile, String)> {
    for tag in tags {
        let normalized = tag.to_ascii_lowercase().replace('-', "_");
        match normalized.as_str() {
            "http_endpoint" | "http-endpoint" => {
                return Some((
                    DeploymentProfile::HttpEndpoint,
                    "inferred from tag http-endpoint".to_string(),
                ));
            }
            "scheduled" | "cron" | "scheduled_source" => {
                return Some((
                    DeploymentProfile::ScheduledSource,
                    "inferred from tag scheduled".to_string(),
                ));
            }
            "queue_consumer" | "queue-consumer" => {
                return Some((
                    DeploymentProfile::QueueConsumer,
                    "inferred from tag queue-consumer".to_string(),
                ));
            }
            "long_lived" | "long-lived" | "long_lived_service" | "long-lived-service" => {
                return Some((
                    DeploymentProfile::LongLivedService,
                    "inferred from tag long-lived".to_string(),
                ));
            }
            "one_shot" | "one-shot" | "one_shot_job" | "one-shot-job" => {
                return Some((
                    DeploymentProfile::OneShotJob,
                    "inferred from tag one-shot".to_string(),
                ));
            }
            _ => {}
        }
    }
    None
}

fn profile_from_world(world: &str) -> Option<(DeploymentProfile, String)> {
    let lowered = world.to_ascii_lowercase();
    if lowered.contains("http") || lowered.contains("webhook") {
        return Some((
            DeploymentProfile::HttpEndpoint,
            format!("inferred from world '{world}'"),
        ));
    }
    if lowered.contains("schedule") || lowered.contains("timer") || lowered.contains("cron") {
        return Some((
            DeploymentProfile::ScheduledSource,
            format!("inferred from world '{world}'"),
        ));
    }
    if lowered.contains("queue") || lowered.contains("consumer") || lowered.contains("sink") {
        return Some((
            DeploymentProfile::QueueConsumer,
            format!("inferred from world '{world}'"),
        ));
    }
    if lowered.contains("worker") || lowered.contains("job") {
        return Some((
            DeploymentProfile::OneShotJob,
            format!("inferred from world '{world}'"),
        ));
    }
    None
}

fn default_profile(role: &ComponentRole) -> (DeploymentProfile, bool) {
    match role {
        ComponentRole::Worker => (DeploymentProfile::OneShotJob, false),
        ComponentRole::EventProvider | ComponentRole::EventBridge => {
            (DeploymentProfile::LongLivedService, true)
        }
        ComponentRole::MessagingAdapter => (DeploymentProfile::LongLivedService, true),
        ComponentRole::Other => (DeploymentProfile::LongLivedService, true),
    }
}

fn parse_profile(value: &str) -> Option<DeploymentProfile> {
    let normalized = value.trim().to_ascii_lowercase().replace(['-', ' '], "_");
    match normalized.as_str() {
        "longlivedservice" | "long_lived_service" => Some(DeploymentProfile::LongLivedService),
        "httpendpoint" | "http_endpoint" => Some(DeploymentProfile::HttpEndpoint),
        "queueconsumer" | "queue_consumer" => Some(DeploymentProfile::QueueConsumer),
        "scheduledsource" | "scheduled_source" => Some(DeploymentProfile::ScheduledSource),
        "oneshotjob" | "one_shot_job" | "one_shot" => Some(DeploymentProfile::OneShotJob),
        _ => None,
    }
}

fn tags_for_component(annotations: &JsonMap<String, JsonValue>, component_id: &str) -> Vec<String> {
    let mut tags = Vec::new();
    if let Some(entry) = annotations
        .get("greentic.tags")
        .and_then(|value| value.as_object())
        .and_then(|map| map.get(component_id))
    {
        if let Some(values) = entry.as_array() {
            for value in values {
                if let Some(tag) = value.as_str() {
                    tags.push(tag.to_string());
                }
            }
        } else if let Some(tag) = entry.as_str() {
            tags.push(tag.to_string());
        }
    }
    tags
}

fn map_profile_to_infra(target: &Target, profile: &DeploymentProfile) -> InfraPlan {
    let (summary, resources) = match (target, profile) {
        (Target::Local, DeploymentProfile::HttpEndpoint) => (
            "local gateway + handler".to_string(),
            vec!["local-gateway".into(), "runner-handler".into()],
        ),
        (Target::Aws, DeploymentProfile::HttpEndpoint) => (
            "api-gateway + lambda".to_string(),
            vec!["api-gateway".into(), "lambda".into()],
        ),
        (Target::Azure, DeploymentProfile::HttpEndpoint) => (
            "function app (http trigger)".to_string(),
            vec!["function-app".into()],
        ),
        (Target::Gcp, DeploymentProfile::HttpEndpoint) => {
            ("cloud run (http)".to_string(), vec!["cloud-run".into()])
        }
        (Target::K8s, DeploymentProfile::HttpEndpoint) => (
            "ingress + service + deployment".to_string(),
            vec!["ingress".into(), "service".into(), "deployment".into()],
        ),
        (Target::Local, DeploymentProfile::LongLivedService) => (
            "runner-managed long-lived process".to_string(),
            vec!["local-runner".into()],
        ),
        (Target::Aws, DeploymentProfile::LongLivedService) => (
            "ecs/eks service".to_string(),
            vec!["container-service".into()],
        ),
        (Target::Azure, DeploymentProfile::LongLivedService) => (
            "container apps / app service".to_string(),
            vec!["container-app".into()],
        ),
        (Target::Gcp, DeploymentProfile::LongLivedService) => (
            "cloud run (always on)".to_string(),
            vec!["cloud-run".into()],
        ),
        (Target::K8s, DeploymentProfile::LongLivedService) => (
            "deployment + service".to_string(),
            vec!["deployment".into(), "service".into()],
        ),
        (Target::Local, DeploymentProfile::QueueConsumer) => (
            "local queue worker".to_string(),
            vec!["local-queue-worker".into()],
        ),
        (Target::Aws, DeploymentProfile::QueueConsumer) => (
            "sqs/event source + lambda".to_string(),
            vec!["sqs".into(), "lambda".into()],
        ),
        (Target::Azure, DeploymentProfile::QueueConsumer) => (
            "service bus queue trigger".to_string(),
            vec!["service-bus".into(), "function".into()],
        ),
        (Target::Gcp, DeploymentProfile::QueueConsumer) => (
            "pubsub subscriber".to_string(),
            vec!["pubsub".into(), "subscriber".into()],
        ),
        (Target::K8s, DeploymentProfile::QueueConsumer) => (
            "deployment + queue consumer".to_string(),
            vec!["deployment".into()],
        ),
        (Target::Local, DeploymentProfile::ScheduledSource) => (
            "local scheduler + runner invocation".to_string(),
            vec!["scheduler".into(), "runner".into()],
        ),
        (Target::Aws, DeploymentProfile::ScheduledSource) => (
            "eventbridge schedule + lambda".to_string(),
            vec!["eventbridge".into(), "lambda".into()],
        ),
        (Target::Azure, DeploymentProfile::ScheduledSource) => (
            "timer-triggered function".to_string(),
            vec!["function-app".into()],
        ),
        (Target::Gcp, DeploymentProfile::ScheduledSource) => (
            "cloud scheduler + run/function".to_string(),
            vec!["cloud-scheduler".into(), "cloud-run".into()],
        ),
        (Target::K8s, DeploymentProfile::ScheduledSource) => {
            ("cronjob".to_string(), vec!["cronjob".into()])
        }
        (Target::Local, DeploymentProfile::OneShotJob) => {
            ("runner one-shot job".to_string(), vec!["runner".into()])
        }
        (Target::Aws, DeploymentProfile::OneShotJob) => {
            ("lambda invocation".to_string(), vec!["lambda".into()])
        }
        (Target::Azure, DeploymentProfile::OneShotJob) => (
            "container apps job / function".to_string(),
            vec!["container-app-job".into()],
        ),
        (Target::Gcp, DeploymentProfile::OneShotJob) => {
            ("cloud run job".to_string(), vec!["cloud-run-job".into()])
        }
        (Target::K8s, DeploymentProfile::OneShotJob) => ("job".to_string(), vec!["job".into()]),
    };

    InfraPlan {
        target: target.clone(),
        profile: profile.clone(),
        summary,
        resources,
        notes: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Action, DeployerConfig, OutputFormat, Provider};
    use crate::iac::IaCTool;
    use crate::plan::Target;
    use crate::plan::{ComponentRole, DeploymentProfile};
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn builds_plan_from_example_pack() {
        let config = DeployerConfig {
            action: Action::Plan,
            provider: Provider::Aws,
            strategy: "iac-only".into(),
            tenant: "acme".into(),
            environment: "staging".into(),
            pack_path: PathBuf::from("examples/acme-pack"),
            yes: true,
            preview: false,
            dry_run: false,
            iac_tool: IaCTool::Terraform,
            output: OutputFormat::Text,
        };

        let plan = build_plan(&config).expect("should build plan");
        assert_eq!(plan.plan.tenant, "acme");
        assert_eq!(plan.plan.environment, "staging");
        assert_eq!(plan.plan.runners.len(), 1);
        assert_eq!(plan.plan.secrets.len(), 2);
        assert_eq!(plan.plan.oauth.len(), 2);
        assert_eq!(plan.deployment.provider, "aws");
        assert_eq!(plan.deployment.strategy, "iac-only");
        assert_eq!(plan.target, Target::Aws);
        assert!(
            !plan.components.is_empty(),
            "expected component planning entries"
        );
    }

    #[test]
    fn builds_plan_from_complex_pack() {
        let config = DeployerConfig {
            action: Action::Plan,
            provider: Provider::Azure,
            strategy: "iac-only".into(),
            tenant: "acmeplus".into(),
            environment: "staging".into(),
            pack_path: PathBuf::from("examples/acme-plus-pack"),
            yes: true,
            preview: false,
            dry_run: false,
            iac_tool: IaCTool::Terraform,
            output: OutputFormat::Text,
        };

        let plan = build_plan(&config).expect("should build complex plan");
        assert_eq!(plan.plan.tenant, "acmeplus");
        assert!(plan.plan.messaging.is_some(), "expected messaging subjects");
        assert!(
            plan.plan.secrets.len() >= 4,
            "expected merged secrets from annotations and components"
        );
        assert!(
            plan.plan.channels.len() >= 2,
            "expected channel entries from connectors"
        );
        assert_eq!(plan.plan.oauth.len(), 2);
        assert_eq!(plan.deployment.provider, "azure");
        assert_eq!(plan.deployment.strategy, "iac-only");
        assert_eq!(plan.target, Target::Azure);
    }

    #[test]
    fn deployment_hints_respect_annotations() {
        let mut annotations = JsonMap::new();
        annotations.insert(
            "greentic.deployment".into(),
            json!({ "provider": "k8s", "strategy": "kubectl" }),
        );
        let config = DeployerConfig {
            action: Action::Plan,
            provider: Provider::Aws,
            strategy: "iac-only".into(),
            tenant: "acme".into(),
            environment: "dev".into(),
            pack_path: PathBuf::from("examples/acme-pack"),
            yes: true,
            preview: false,
            dry_run: false,
            iac_tool: IaCTool::Terraform,
            output: OutputFormat::Text,
        };
        let hints = super::build_deployment_hints(&annotations, &config);
        assert_eq!(hints.provider, "k8s");
        assert_eq!(hints.strategy, "kubectl");
        assert_eq!(hints.target, Target::K8s);
    }

    #[test]
    fn infers_profile_from_tags_without_warning() {
        let (profile, notes) = infer_profile(
            "comp-tagged",
            None,
            &ComponentRole::EventProvider,
            Some("greentic:events/source"),
            &[String::from("http-endpoint")],
        );
        assert_eq!(profile, DeploymentProfile::HttpEndpoint);
        let notes = notes.expect("notes present");
        assert!(notes.warnings.is_empty());
        assert!(
            notes.source.contains("tag"),
            "expected tag-based inference source"
        );
    }

    #[test]
    fn defaults_profile_with_warning_when_missing_hints() {
        let (profile, notes) = infer_profile(
            "comp-unhinted",
            None,
            &ComponentRole::EventBridge,
            None,
            &[],
        );
        assert_eq!(profile, DeploymentProfile::LongLivedService);
        let notes = notes.expect("notes present");
        assert!(
            notes.warnings.iter().any(|w| w.contains("comp-unhinted")),
            "should emit warning mentioning component id"
        );
    }

    #[test]
    fn maps_profiles_to_all_targets() {
        let profile = DeploymentProfile::ScheduledSource;
        for target in [
            Target::Local,
            Target::Aws,
            Target::Azure,
            Target::Gcp,
            Target::K8s,
        ] {
            let infra = map_profile_to_infra(&target, &profile);
            assert_eq!(infra.target, target);
            assert!(
                !infra.summary.is_empty(),
                "infra summary should be present for {target:?}"
            );
        }
    }
}
