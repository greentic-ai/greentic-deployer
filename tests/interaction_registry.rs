use greentic_deployer::bootstrap::capabilities::build_host_capabilities;
use greentic_deployer::bootstrap::interaction::{
    InteractionAdapterKind, InteractionPolicy, adapters_for_mode,
};
use greentic_deployer::bootstrap::network::{NetAllowList, NetworkPolicy};
use greentic_deployer::config::InteractionMode;

#[test]
fn auto_mode_includes_http_when_allowed() {
    let policy = InteractionPolicy {
        allow_listeners: true,
        allow_network: true,
        offline_only: false,
        allowlist_configured: true,
    };
    let adapters = adapters_for_mode(InteractionMode::Auto, &policy);
    assert!(adapters.contains(&InteractionAdapterKind::Cli));
    assert!(adapters.contains(&InteractionAdapterKind::Json));
    assert!(adapters.contains(&InteractionAdapterKind::Http));
    assert!(adapters.contains(&InteractionAdapterKind::Mqtt));
}

#[test]
fn no_listeners_filters_http_and_mqtt() {
    let network = NetworkPolicy::new(false, false, NetAllowList::default());
    let caps = build_host_capabilities(InteractionMode::Auto, false, &network);
    assert!(caps.adapters.contains(&InteractionAdapterKind::Cli));
    assert!(caps.adapters.contains(&InteractionAdapterKind::Json));
    assert!(!caps.adapters.contains(&InteractionAdapterKind::Http));
    assert!(!caps.adapters.contains(&InteractionAdapterKind::Mqtt));
}

#[test]
fn json_mode_restricts_to_json_adapter() {
    let policy = InteractionPolicy {
        allow_listeners: true,
        allow_network: true,
        offline_only: false,
        allowlist_configured: true,
    };
    let adapters = adapters_for_mode(InteractionMode::Json, &policy);
    assert_eq!(adapters, vec![InteractionAdapterKind::Json]);
}

#[test]
fn http_mode_requires_policy() {
    let policy = InteractionPolicy {
        allow_listeners: false,
        allow_network: false,
        offline_only: false,
        allowlist_configured: false,
    };
    let adapters = adapters_for_mode(InteractionMode::Http, &policy);
    assert!(adapters.is_empty());

    let policy_on = InteractionPolicy {
        allow_listeners: true,
        allow_network: true,
        offline_only: false,
        allowlist_configured: true,
    };
    let adapters_on = adapters_for_mode(InteractionMode::Http, &policy_on);
    assert_eq!(adapters_on, vec![InteractionAdapterKind::Http]);
}

#[test]
fn listeners_disabled_by_default() {
    let network = NetworkPolicy::new(false, false, NetAllowList::default());
    let caps = build_host_capabilities(InteractionMode::Auto, false, &network);
    assert!(!caps.adapters.contains(&InteractionAdapterKind::Http));
    assert!(!caps.adapters.contains(&InteractionAdapterKind::Mqtt));
    assert!(!caps.disabled_reasons.is_empty());
}

#[test]
fn listeners_available_when_enabled() {
    let allowlist = NetAllowList::parse(Some("mqtt.example.com")).expect("allowlist");
    let network = NetworkPolicy::new(true, false, allowlist);
    let caps = build_host_capabilities(InteractionMode::Auto, true, &network);
    assert!(caps.adapters.contains(&InteractionAdapterKind::Http));
    assert!(caps.adapters.contains(&InteractionAdapterKind::Mqtt));
}
