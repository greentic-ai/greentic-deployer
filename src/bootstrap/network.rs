use std::net::IpAddr;

use ipnet::IpNet;

use crate::error::{DeployerError, Result};

#[derive(Debug, Clone, Default)]
pub struct NetAllowList {
    entries: Vec<NetEntry>,
}

#[derive(Debug, Clone)]
enum NetEntry {
    Host(String),
    Cidr(IpNet),
}

impl NetAllowList {
    pub fn parse(raw: Option<&str>) -> Result<Self> {
        let mut entries = Vec::new();
        if let Some(raw) = raw {
            for token in raw.split(',') {
                let entry = token.trim();
                if entry.is_empty() {
                    continue;
                }
                if let Ok(net) = entry.parse::<IpNet>() {
                    entries.push(NetEntry::Cidr(net));
                    continue;
                }

                let host = normalize_host(entry);
                if host.is_empty() {
                    continue;
                }
                // Allow CIDR parsed after stripping scheme/port.
                if let Ok(net) = host.parse::<IpNet>() {
                    entries.push(NetEntry::Cidr(net));
                    continue;
                }
                entries.push(NetEntry::Host(host.to_ascii_lowercase()));
            }
        }
        Ok(Self { entries })
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn is_allowed(&self, target: &str) -> bool {
        let host = normalize_host(target);
        let host_lower = host.to_ascii_lowercase();
        let ip = host.parse::<IpAddr>().ok();

        for entry in &self.entries {
            match entry {
                NetEntry::Host(allowed) => {
                    if &host_lower == allowed {
                        return true;
                    }
                }
                NetEntry::Cidr(net) => {
                    if let Some(ip) = ip
                        && net.contains(&ip)
                    {
                        return true;
                    }
                }
            }
        }
        false
    }
}

#[derive(Debug, Clone)]
pub struct NetworkPolicy {
    allow_network: bool,
    offline_only: bool,
    allowlist: NetAllowList,
}

impl NetworkPolicy {
    pub fn new(allow_network: bool, offline_only: bool, allowlist: NetAllowList) -> Self {
        Self {
            allow_network,
            offline_only,
            allowlist,
        }
    }

    pub fn allow_network(&self) -> bool {
        self.allow_network
    }

    pub fn offline_only(&self) -> bool {
        self.offline_only
    }

    pub fn allowlist_configured(&self) -> bool {
        !self.allowlist.is_empty()
    }

    pub fn enforce(&self, target: &str) -> Result<()> {
        if self.offline_only {
            return Err(DeployerError::Other(
                "offline-only mode blocks outbound network access".into(),
            ));
        }
        if !self.allow_network {
            return Err(DeployerError::Other(
                "network access disabled; pass --allow-network to enable outbound calls".into(),
            ));
        }
        if !self.allowlist.is_allowed(target) {
            return Err(DeployerError::Other(format!(
                "network target '{target}' not in allowlist; set --net-allowlist to permit it"
            )));
        }
        Ok(())
    }
}

fn normalize_host(input: &str) -> String {
    // Remove scheme if present.
    let without_scheme = if let Some(pos) = input.find("://") {
        &input[pos + 3..]
    } else {
        input
    };

    // Drop path/query fragments.
    let without_path = without_scheme.split('/').next().unwrap_or_default();
    let without_userinfo = without_path.split('@').next_back().unwrap_or_default();

    // Strip brackets for IPv6 and drop port if present.
    let trimmed = without_userinfo.trim_matches(['[', ']']);
    trimmed.split(':').next().unwrap_or_default().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_matches_hosts_and_cidrs() {
        let allowlist =
            NetAllowList::parse(Some("example.com, 10.0.0.0/8, https://api.test:443/path"))
                .expect("parse allowlist");
        assert!(allowlist.is_allowed("example.com"));
        assert!(allowlist.is_allowed("http://example.com:8080/service"));
        assert!(allowlist.is_allowed("10.1.2.3"));
        assert!(allowlist.is_allowed("https://api.test/resource"));
        assert!(!allowlist.is_allowed("other.com"));
        assert!(!allowlist.is_allowed("192.168.1.10"));
    }

    #[test]
    fn network_policy_enforces_ordering() {
        let allowlist = NetAllowList::parse(Some("broker.local")).expect("parse");
        let policy = NetworkPolicy::new(true, false, allowlist);
        assert!(policy.enforce("mqtt://broker.local:1883").is_ok());

        let blocked = NetworkPolicy::new(true, false, NetAllowList::default());
        assert!(blocked.enforce("broker.local").is_err());

        let offline = NetworkPolicy::new(true, true, NetAllowList::default());
        assert!(offline.enforce("broker.local").is_err());

        let disabled = NetworkPolicy::new(false, false, NetAllowList::default());
        assert!(disabled.enforce("broker.local").is_err());
    }
}
