use std::collections::BTreeMap;
use std::env;

use serde::{Deserialize, Serialize};

use greentic_types::deployment::DeploymentPlan;

use crate::config::DeployerConfig;

/// Provider-agnostic deployment plan bundle enriched with deployer hints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanContext {
    /// Canonical plan produced by `greentic-types`.
    pub plan: DeploymentPlan,
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
            "Plan for {} @ {}: {} runners, {} channels, {} secrets, {} oauth clients",
            self.plan.tenant,
            self.plan.environment,
            self.plan.runners.len(),
            self.plan.channels.len(),
            self.secrets.len(),
            self.plan.oauth.len()
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
) -> PlanContext {
    let telemetry = build_telemetry_context(&plan, config);
    let messaging = build_messaging_context(&plan);
    let channels = build_channel_context(&plan, config);
    let secrets = build_secret_context(&plan);
    PlanContext {
        plan,
        messaging,
        telemetry,
        channels,
        secrets,
        deployment,
    }
}
