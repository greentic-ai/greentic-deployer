use crate::config::InteractionMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionAdapterKind {
    Cli,
    Json,
    Http,
    Mqtt,
}

#[derive(Debug, Clone, Copy)]
pub struct InteractionPolicy {
    pub allow_listeners: bool,
    pub allow_network: bool,
    pub offline_only: bool,
    pub allowlist_configured: bool,
}

pub trait InteractionAdapter {
    fn id(&self) -> &'static str;
    fn kind(&self) -> InteractionAdapterKind;
    fn is_available(&self, policy: &InteractionPolicy) -> bool;
}

#[derive(Debug)]
struct CliAdapter;

impl InteractionAdapter for CliAdapter {
    fn id(&self) -> &'static str {
        "cli"
    }

    fn kind(&self) -> InteractionAdapterKind {
        InteractionAdapterKind::Cli
    }

    fn is_available(&self, _policy: &InteractionPolicy) -> bool {
        true
    }
}

#[derive(Debug)]
struct JsonAdapter;

impl InteractionAdapter for JsonAdapter {
    fn id(&self) -> &'static str {
        "json"
    }

    fn kind(&self) -> InteractionAdapterKind {
        InteractionAdapterKind::Json
    }

    fn is_available(&self, _policy: &InteractionPolicy) -> bool {
        true
    }
}

#[derive(Debug)]
struct HttpAdapter;

impl InteractionAdapter for HttpAdapter {
    fn id(&self) -> &'static str {
        "http"
    }

    fn kind(&self) -> InteractionAdapterKind {
        InteractionAdapterKind::Http
    }

    fn is_available(&self, policy: &InteractionPolicy) -> bool {
        policy.allow_listeners && policy.allow_network && !policy.offline_only
    }
}

#[derive(Debug)]
struct MqttAdapter;

impl InteractionAdapter for MqttAdapter {
    fn id(&self) -> &'static str {
        "mqtt"
    }

    fn kind(&self) -> InteractionAdapterKind {
        InteractionAdapterKind::Mqtt
    }

    fn is_available(&self, policy: &InteractionPolicy) -> bool {
        policy.allow_listeners
            && policy.allow_network
            && policy.allowlist_configured
            && !policy.offline_only
    }
}

fn registry() -> Vec<Box<dyn InteractionAdapter + Send + Sync>> {
    vec![
        Box::new(CliAdapter),
        Box::new(JsonAdapter),
        Box::new(HttpAdapter),
        Box::new(MqttAdapter),
    ]
}

pub fn adapters_for_mode(
    mode: InteractionMode,
    policy: &InteractionPolicy,
) -> Vec<InteractionAdapterKind> {
    let adapters = registry();
    let available: Vec<InteractionAdapterKind> = adapters
        .iter()
        .filter(|a| a.is_available(policy))
        .map(|a| a.kind())
        .collect();

    match mode {
        InteractionMode::Cli => available
            .into_iter()
            .filter(|k| *k == InteractionAdapterKind::Cli)
            .collect(),
        InteractionMode::Json => available
            .into_iter()
            .filter(|k| *k == InteractionAdapterKind::Json)
            .collect(),
        InteractionMode::Auto => available,
        InteractionMode::Http => available
            .into_iter()
            .filter(|k| *k == InteractionAdapterKind::Http)
            .collect(),
        InteractionMode::Mqtt => available
            .into_iter()
            .filter(|k| *k == InteractionAdapterKind::Mqtt)
            .collect(),
    }
}
