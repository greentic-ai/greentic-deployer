use std::collections::BTreeMap;
use std::env;

use serde::{Deserialize, Serialize};

use greentic_types::deployment::DeploymentPlan;

use crate::config::{DeployerConfig, Provider};

/// Generic component role derived from pack metadata and WIT worlds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentRole {
    EventProvider,
    EventBridge,
    MessagingAdapter,
    Worker,
    Other,
}

impl ComponentRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            ComponentRole::EventProvider => "event_provider",
            ComponentRole::EventBridge => "event_bridge",
            ComponentRole::MessagingAdapter => "messaging_adapter",
            ComponentRole::Worker => "worker",
            ComponentRole::Other => "other",
        }
    }
}

/// Abstract deployment profile used to map onto target infrastructure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentProfile {
    LongLivedService,
    HttpEndpoint,
    QueueConsumer,
    ScheduledSource,
    OneShotJob,
}

impl DeploymentProfile {
    pub fn as_str(&self) -> &'static str {
        match self {
            DeploymentProfile::LongLivedService => "long_lived_service",
            DeploymentProfile::HttpEndpoint => "http_endpoint",
            DeploymentProfile::QueueConsumer => "queue_consumer",
            DeploymentProfile::ScheduledSource => "scheduled_source",
            DeploymentProfile::OneShotJob => "one_shot_job",
        }
    }
}

/// Supported deployment targets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Local,
    Aws,
    Azure,
    Gcp,
    K8s,
}

impl Target {
    pub fn as_str(&self) -> &'static str {
        match self {
            Target::Local => "local",
            Target::Aws => "aws",
            Target::Azure => "azure",
            Target::Gcp => "gcp",
            Target::K8s => "k8s",
        }
    }
}

impl From<Provider> for Target {
    fn from(value: Provider) -> Self {
        match value {
            Provider::Local => Target::Local,
            Provider::Aws => Target::Aws,
            Provider::Azure => Target::Azure,
            Provider::Gcp => Target::Gcp,
            Provider::K8s => Target::K8s,
        }
    }
}

/// Per-component planning entry with inferred role/profile and infra summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedComponent {
    pub id: String,
    pub role: ComponentRole,
    pub profile: DeploymentProfile,
    pub target: Target,
    pub infra: InfraPlan,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inference: Option<InferenceNotes>,
}

/// Target-specific infrastructure mapping for a component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfraPlan {
    pub target: Target,
    pub profile: DeploymentProfile,
    /// Short human-readable mapping summary (e.g. "api-gateway + lambda").
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// Inference details attached to a component entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceNotes {
    pub source: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Provider-agnostic deployment plan bundle enriched with deployer hints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanContext {
    /// Canonical plan produced by `greentic-types`.
    pub plan: DeploymentPlan,
    /// Target selected for planning/rendering.
    pub target: Target,
    /// Per-component deployment mapping (role + profile).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<PlannedComponent>,
    /// Messaging hints inferred from tenant/environment.
    pub messaging: MessagingContext,
    /// Telemetry hints applied to generated artifacts.
    pub telemetry: TelemetryContext,
    /// Channel ingress and OAuth hints.
    pub channels: Vec<ChannelContext>,
    /// Logical secrets referenced by the deployment.
    pub secrets: Vec<SecretContext>,
    /// Deployment target hints (provider/strategy strings).
    pub deployment: DeploymentHints,
}

impl PlanContext {
    /// Returns a compact summary string for CLI output.
    pub fn summary(&self) -> String {
        format!(
            "Plan for {} @ {} (target {}): {} runners, {} channels, {} secrets, {} oauth clients, {} components",
            self.plan.tenant,
            self.plan.environment,
            self.target.as_str(),
            self.plan.runners.len(),
            self.plan.channels.len(),
            self.secrets.len(),
            self.plan.oauth.len(),
            self.components.len()
        )
    }
}

/// Derived NATS/messaging hints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagingContext {
    pub logical_cluster: String,
    pub replicas: u16,
    pub admin_url: String,
}

/// Derived telemetry hints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryContext {
    pub otlp_endpoint: String,
    pub resource_attributes: BTreeMap<String, String>,
}

/// Channel ingress hints for IaC rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelContext {
    pub name: String,
    pub kind: String,
    pub ingress: Vec<String>,
    pub oauth_required: bool,
}

/// Secret metadata surfaced during apply/destroy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretContext {
    pub key: String,
    pub scope: String,
}

/// Deployment hints used to resolve provider/strategy dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentHints {
    pub target: Target,
    pub provider: String,
    pub strategy: String,
}

/// Builds carrier resource attributes used by telemetry-aware deployments.
pub fn build_telemetry_context(plan: &DeploymentPlan, config: &DeployerConfig) -> TelemetryContext {
    let endpoint = plan
        .telemetry
        .as_ref()
        .and_then(|t| t.suggested_endpoint.clone())
        .or_else(|| env::var("GREENTIC_OTLP_ENDPOINT").ok())
        .or_else(|| env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok())
        .unwrap_or_else(|| "https://otel.greentic.ai".to_string());

    let mut resource_attributes = BTreeMap::new();
    resource_attributes.insert(
        "service.name".to_string(),
        format!("greentic-deployer-{}", config.provider.as_str()),
    );
    resource_attributes.insert(
        "deployment.environment".to_string(),
        config.environment.clone(),
    );
    resource_attributes.insert("greentic.tenant".to_string(), config.tenant.clone());

    TelemetryContext {
        otlp_endpoint: endpoint,
        resource_attributes,
    }
}

/// Builds channel ingress hints based on the plan data and CLI config.
pub fn build_channel_context(
    plan: &DeploymentPlan,
    config: &DeployerConfig,
) -> Vec<ChannelContext> {
    let base_domain =
        env::var("GREENTIC_BASE_DOMAIN").unwrap_or_else(|_| "deploy.greentic.ai".to_string());
    plan.channels
        .iter()
        .map(|channel| {
            let ingress = format!(
                "https://{}/ingress/{}/{}/{}",
                base_domain, config.environment, config.tenant, channel.kind
            );
            ChannelContext {
                name: channel.name.clone(),
                kind: channel.kind.clone(),
                ingress: vec![ingress],
                oauth_required: matches!(
                    channel.kind.as_str(),
                    "slack" | "teams" | "webex" | "telegram" | "whatsapp"
                ),
            }
        })
        .collect()
}

/// Builds secret hints for manifest output.
pub fn build_secret_context(plan: &DeploymentPlan) -> Vec<SecretContext> {
    plan.secrets
        .iter()
        .map(|secret| SecretContext {
            key: secret.key.clone(),
            scope: secret.scope.clone(),
        })
        .collect()
}

/// Builds messaging hints using tenant/environment heuristics.
pub fn build_messaging_context(plan: &DeploymentPlan) -> MessagingContext {
    let logical_cluster = plan
        .messaging
        .as_ref()
        .map(|m| m.logical_cluster.clone())
        .unwrap_or_else(|| format!("nats-{}-{}", plan.environment, plan.tenant));
    let replicas = if plan.environment.contains("prod") {
        3
    } else {
        1
    };
    let admin_url = format!("https://nats.{}.{}.svc", plan.environment, plan.tenant);

    MessagingContext {
        logical_cluster,
        replicas,
        admin_url,
    }
}

/// Creates a [`PlanContext`] bundle from the base deployment plan.
pub fn assemble_plan(
    plan: DeploymentPlan,
    config: &DeployerConfig,
    deployment: DeploymentHints,
    components: Vec<PlannedComponent>,
) -> PlanContext {
    let telemetry = build_telemetry_context(&plan, config);
    let messaging = build_messaging_context(&plan);
    let channels = build_channel_context(&plan, config);
    let secrets = build_secret_context(&plan);
    PlanContext {
        plan,
        target: deployment.target.clone(),
        components,
        messaging,
        telemetry,
        channels,
        secrets,
        deployment,
    }
}
