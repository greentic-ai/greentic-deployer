use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;

use serde::{Deserialize, Serialize};

use crate::config::DeployerConfig;
use greentic_flow::FlowBundle;
use greentic_oauth_core::ProviderId;
use greentic_pack::builder::PackManifest;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentPlan {
    pub tenant: String,
    pub environment: String,
    pub pack_id: String,
    pub pack_version: String,
    pub flows: Vec<FlowSummary>,
    pub messaging: MessagingPlan,
    pub runners: Vec<RunnerServicePlan>,
    pub channels: Vec<ChannelServicePlan>,
    pub secrets: Vec<SecretSpec>,
    pub oauth_clients: Vec<OAuthClientSpec>,
    pub telemetry: TelemetryPlan,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowSummary {
    pub id: String,
    pub kind: String,
    pub entry: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagingPlan {
    pub nats: NatsPlan,
    pub subjects: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatsPlan {
    pub cluster_name: String,
    pub replicas: u16,
    pub enable_jetstream: bool,
    pub admin_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerServicePlan {
    pub name: String,
    pub components: Vec<String>,
    pub resources: ResourceHints,
    pub bindings: Vec<BindingHint>,
    pub wasi_world: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindingHint {
    pub name: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceHints {
    pub cpu_millis: u32,
    pub memory_mb: u32,
}

impl Default for ResourceHints {
    fn default() -> Self {
        Self {
            cpu_millis: 500,
            memory_mb: 512,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelServicePlan {
    pub name: String,
    pub channel_type: String,
    pub ingress: Vec<IngressEndpoint>,
    pub oauth_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngressEndpoint {
    pub url: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretSpec {
    pub name: String,
    pub description: Option<String>,
    pub required_for: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthClientSpec {
    pub provider: ProviderId,
    pub scopes: Vec<String>,
    pub redirect_urls: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryPlan {
    pub otlp_endpoint: String,
    pub resource_attributes: BTreeMap<String, String>,
}

impl DeploymentPlan {
    pub fn summary(&self) -> String {
        format!(
            "Plan for {} @ {}: {} flows, {} runners, {} channels, {} secrets",
            self.tenant,
            self.environment,
            self.flows.len(),
            self.runners.len(),
            self.channels.len(),
            self.secrets.len()
        )
    }

    pub fn from_manifest(
        config: &DeployerConfig,
        manifest: &PackManifest,
        flows: &[FlowBundle],
    ) -> Self {
        let flow_summaries = flows
            .iter()
            .map(|flow| FlowSummary {
                id: flow.id.clone(),
                kind: flow.kind.clone(),
                entry: flow.entry.clone(),
            })
            .collect();

        let channel_types = detect_channel_types(flows);
        let channels = build_channel_services(&channel_types, config);
        let secrets = build_secret_plan(&channel_types, manifest);
        let oauth_clients = build_oauth_plan(&channel_types, manifest, config);
        let messaging = build_messaging_plan(config, flows);

        let runners = build_runner_plan(manifest, &channel_types);

        let telemetry = TelemetryPlan::new(config);

        Self {
            tenant: config.tenant.clone(),
            environment: config.environment.clone(),
            pack_id: manifest.meta.pack_id.clone(),
            pack_version: manifest.meta.version.to_string(),
            flows: flow_summaries,
            messaging,
            runners,
            channels,
            secrets,
            oauth_clients,
            telemetry,
        }
    }
}

fn detect_channel_types(flows: &[FlowBundle]) -> Vec<String> {
    let mut types = HashSet::new();

    for flow in flows {
        if !flow.kind.is_empty() {
            types.insert(flow.kind.clone());
        }
        for node in &flow.nodes {
            if let Some(channel) = channel_from_component(&node.component.name) {
                types.insert(channel.to_owned());
            }
        }
    }

    if types.is_empty() {
        types.insert("messaging".to_string());
    }

    let mut list: Vec<_> = types.into_iter().collect();
    list.sort();
    list
}

fn channel_from_component(component: &str) -> Option<&'static str> {
    let lower = component.to_ascii_lowercase();
    if lower.contains("slack") {
        Some("slack")
    } else if lower.contains("teams") {
        Some("teams")
    } else if lower.contains("webex") {
        Some("webex")
    } else if lower.contains("webchat") {
        Some("webchat")
    } else if lower.contains("whatsapp") {
        Some("whatsapp")
    } else if lower.contains("telegram") {
        Some("telegram")
    } else if lower.contains("messaging") {
        Some("messaging")
    } else {
        None
    }
}

fn build_channel_services(types: &[String], config: &DeployerConfig) -> Vec<ChannelServicePlan> {
    let base_domain =
        env::var("GREENTIC_BASE_DOMAIN").unwrap_or_else(|_| "deploy.greentic.ai".to_string());

    types
        .iter()
        .map(|channel| ChannelServicePlan {
            name: format!("channel-{}", channel),
            channel_type: channel.clone(),
            ingress: vec![IngressEndpoint {
                url: format!(
                    "https://{base_domain}/ingress/{}/{}/{}",
                    config.environment, config.tenant, channel
                ),
                description: Some("public ingress endpoint".to_string()),
            }],
            oauth_required: requires_oauth(channel),
        })
        .collect()
}

fn requires_oauth(channel: &str) -> bool {
    matches!(
        channel,
        "slack" | "teams" | "webex" | "telegram" | "whatsapp"
    )
}

fn build_secret_plan(types: &[String], manifest: &PackManifest) -> Vec<SecretSpec> {
    let mut specs: HashMap<String, SecretSpec> = HashMap::new();

    for channel in types {
        match channel.as_str() {
            "slack" => {
                specs
                    .entry("SLACK_BOT_TOKEN".to_string())
                    .or_insert_with(|| SecretSpec {
                        name: "SLACK_BOT_TOKEN".to_string(),
                        description: Some("Bot token used by Slack runners".to_string()),
                        required_for: vec!["slack".to_string()],
                    });
                specs
                    .entry("SLACK_SIGNING_SECRET".to_string())
                    .or_insert_with(|| SecretSpec {
                        name: "SLACK_SIGNING_SECRET".to_string(),
                        description: Some(
                            "Slack signing secret for ingress verification".to_string(),
                        ),
                        required_for: vec!["slack".to_string()],
                    });
            }
            "teams" => {
                specs
                    .entry("TEAMS_CLIENT_ID".to_string())
                    .or_insert_with(|| SecretSpec {
                        name: "TEAMS_CLIENT_ID".to_string(),
                        description: Some("Microsoft Teams OAuth client id".to_string()),
                        required_for: vec!["teams".to_string()],
                    });
                specs
                    .entry("TEAMS_CLIENT_SECRET".to_string())
                    .or_insert_with(|| SecretSpec {
                        name: "TEAMS_CLIENT_SECRET".to_string(),
                        description: Some("Microsoft Teams OAuth client secret".to_string()),
                        required_for: vec!["teams".to_string()],
                    });
            }
            "webex" => {
                specs
                    .entry("WEBEX_BOT_TOKEN".to_string())
                    .or_insert_with(|| SecretSpec {
                        name: "WEBEX_BOT_TOKEN".to_string(),
                        description: Some("Webex bot token".to_string()),
                        required_for: vec!["webex".to_string()],
                    });
            }
            "webchat" => {
                specs
                    .entry("WEBCHAT_API_KEY".to_string())
                    .or_insert_with(|| SecretSpec {
                        name: "WEBCHAT_API_KEY".to_string(),
                        description: Some("Webchat platform API key".to_string()),
                        required_for: vec!["webchat".to_string()],
                    });
            }
            "whatsapp" => {
                specs
                    .entry("WHATSAPP_API_KEY".to_string())
                    .or_insert_with(|| SecretSpec {
                        name: "WHATSAPP_API_KEY".to_string(),
                        description: Some("WhatsApp Business API credentials".to_string()),
                        required_for: vec!["whatsapp".to_string()],
                    });
            }
            "telegram" => {
                specs
                    .entry("TELEGRAM_BOT_TOKEN".to_string())
                    .or_insert_with(|| SecretSpec {
                        name: "TELEGRAM_BOT_TOKEN".to_string(),
                        description: Some("Telegram bot token".to_string()),
                        required_for: vec!["telegram".to_string()],
                    });
            }
            _ => {
                specs
                    .entry("MESSAGING_NATS_URL".to_string())
                    .or_insert_with(|| SecretSpec {
                        name: "MESSAGING_NATS_URL".to_string(),
                        description: Some("NATS connection URL for messaging".to_string()),
                        required_for: vec!["messaging".to_string()],
                    });
            }
        }
    }

    if let Some(value) = manifest.meta.annotations.get("greentic.secrets") {
        if let Some(map) = value.as_object() {
            for (name, detail) in map {
                specs
                    .entry(name.clone())
                    .and_modify(|spec| {
                        spec.required_for.push("annotated".to_string());
                    })
                    .or_insert_with(|| SecretSpec {
                        name: name.clone(),
                        description: detail
                            .as_str()
                            .map(|s| s.to_string())
                            .or_else(|| Some(detail.to_string())),
                        required_for: vec!["annotated".to_string()],
                    });
            }
        }
    }

    specs.into_values().collect()
}

fn build_oauth_plan(
    types: &[String],
    manifest: &PackManifest,
    config: &DeployerConfig,
) -> Vec<OAuthClientSpec> {
    let mut oauths: HashMap<ProviderId, OAuthClientSpec> = HashMap::new();
    let base_domain =
        env::var("GREENTIC_BASE_DOMAIN").unwrap_or_else(|_| "deploy.greentic.ai".to_string());

    for channel in types {
        if let Some(provider) = oauth_provider(channel) {
            oauths
                .entry(provider.clone())
                .or_insert_with(|| OAuthClientSpec {
                    provider: provider.clone(),
                    scopes: default_scopes(&provider),
                    redirect_urls: vec![format!(
                        "https://{base_domain}/oauth/{}/{}/callback/{}/{}",
                        provider.as_str(),
                        config.tenant,
                        config.environment,
                        provider.as_str(),
                    )],
                });
        }
    }

    if let Some(value) = manifest.meta.annotations.get("greentic.oauth") {
        if let Some(map) = value.as_object() {
            for (provider, detail) in map {
                let scopes = detail
                    .get("scopes")
                    .and_then(|entry| entry.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|v| v.to_string()))
                            .collect()
                    })
                    .unwrap_or_else(Vec::new);
                let provider_id = ProviderId::from_slug(provider);
                oauths
                    .entry(provider_id.clone())
                    .and_modify(|spec| spec.scopes.extend(scopes.clone()))
                    .or_insert_with(|| OAuthClientSpec {
                        provider: provider_id.clone(),
                        scopes,
                        redirect_urls: vec![format!(
                            "https://{base_domain}/oauth/{provider}/callback/{}/{}/{}",
                            config.tenant,
                            config.environment,
                            provider.to_lowercase()
                        )],
                    });
            }
        }
    }

    oauths.into_values().collect()
}

fn oauth_provider(channel: &str) -> Option<ProviderId> {
    let slug = match channel {
        "slack" => "slack",
        "teams" => "microsoft",
        "webex" => "webex",
        "telegram" => "telegram",
        "whatsapp" => "whatsapp",
        other => other,
    };
    Some(ProviderId::from_slug(slug))
}

fn default_scopes(provider: &ProviderId) -> Vec<String> {
    match provider {
        ProviderId::Google => vec!["openid".into(), "email".into(), "profile".into()],
        ProviderId::Microsoft => vec!["User.Read".into(), "Group.Read.All".into()],
        ProviderId::GitHub => vec!["repo".into(), "read:org".into()],
        ProviderId::Custom(name) => vec![format!("{name}:access")],
    }
}

fn build_messaging_plan(config: &DeployerConfig, flows: &[FlowBundle]) -> MessagingPlan {
    let cluster_name = format!("nats-{}-{}", config.environment, config.tenant);
    let replicas = if config.environment.contains("prod") {
        3
    } else {
        1
    };

    let mut subjects = vec![
        format!("messaging.activities.in.{}", config.tenant),
        format!("messaging.activities.out.{}", config.tenant),
        format!("tools.invocations.{}", config.tenant),
    ];

    for flow in flows {
        subjects.push(format!("flows.{}.events", flow.id));
    }

    subjects.sort();
    subjects.dedup();

    MessagingPlan {
        nats: NatsPlan {
            cluster_name,
            replicas,
            enable_jetstream: true,
            admin_url: format!("https://nats.{}.{}.svc", config.environment, config.tenant),
        },
        subjects,
    }
}

fn build_runner_plan(manifest: &PackManifest, channels: &[String]) -> Vec<RunnerServicePlan> {
    manifest
        .components
        .iter()
        .map(|component| {
            let mut bindings = vec![BindingHint {
                name: "nats".to_string(),
                detail: format!(
                    "Connect to {} ({} replicas)",
                    component.name, manifest.meta.pack_id
                ),
            }];

            if !channels.is_empty() {
                bindings.push(BindingHint {
                    name: "channels".to_string(),
                    detail: format!("Handles {}", channels.join(", ")),
                });
            }

            RunnerServicePlan {
                name: format!("{}@{}", component.name, component.version),
                components: vec![component.name.clone()],
                resources: ResourceHints::default(),
                bindings,
                wasi_world: component.world.clone(),
            }
        })
        .collect()
}

impl TelemetryPlan {
    fn new(config: &DeployerConfig) -> Self {
        let endpoint = env::var("GREENTIC_OTLP_ENDPOINT")
            .or_else(|_| env::var("OTEL_EXPORTER_OTLP_ENDPOINT"))
            .unwrap_or_else(|_| "https://otel.greentic.ai".to_string());

        let mut attributes = BTreeMap::new();
        attributes.insert("service.name".to_string(), "greentic-deployer".to_string());
        attributes.insert(
            "deployment.environment".to_string(),
            config.environment.clone(),
        );
        attributes.insert("greentic.tenant".to_string(), config.tenant.clone());

        TelemetryPlan {
            otlp_endpoint: endpoint,
            resource_attributes: attributes,
        }
    }
}
