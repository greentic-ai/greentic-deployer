use crate::bootstrap::interaction::{InteractionAdapterKind, InteractionPolicy, adapters_for_mode};
use crate::bootstrap::network::NetworkPolicy;
use crate::config::InteractionMode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostCapabilities {
    pub adapters: Vec<InteractionAdapterKind>,
    pub no_listeners: bool,
    pub offline_only: bool,
    pub disabled_reasons: Vec<String>,
}

pub fn build_host_capabilities(
    interaction: InteractionMode,
    allow_listeners: bool,
    network_policy: &NetworkPolicy,
) -> HostCapabilities {
    let effective_allow_network = network_policy.allow_network() && !network_policy.offline_only();
    let effective_allow_listeners =
        allow_listeners && !network_policy.offline_only() && effective_allow_network;
    let policy = InteractionPolicy {
        allow_listeners: effective_allow_listeners,
        allow_network: effective_allow_network,
        offline_only: network_policy.offline_only(),
        allowlist_configured: network_policy.allowlist_configured(),
    };
    let adapters = adapters_for_mode(interaction, &policy);

    let mut disabled_reasons = Vec::new();
    if !allow_listeners {
        disabled_reasons.push("listeners not allowed (enable with --allow-listeners)".to_string());
    }
    if network_policy.offline_only() {
        disabled_reasons.push("offline-only mode disables listener adapters".to_string());
    }
    if !network_policy.allow_network() {
        disabled_reasons
            .push("network access disabled; listener adapters require network".to_string());
    }
    if network_policy.allow_network() && !network_policy.allowlist_configured() {
        disabled_reasons.push(
            "network allowlist is empty; outbound adapters remain disabled (set --net-allowlist)"
                .to_string(),
        );
    }
    if !effective_allow_listeners {
        disabled_reasons.push("http/mqtt adapters disabled by policy".to_string());
    }

    HostCapabilities {
        adapters,
        no_listeners: !effective_allow_listeners,
        offline_only: network_policy.offline_only(),
        disabled_reasons,
    }
}
