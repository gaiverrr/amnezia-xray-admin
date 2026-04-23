/// A user as seen in the TUI — merged from server.json + clientsTable
#[derive(Debug, Clone, PartialEq)]
pub struct XrayUser {
    pub uuid: String,
    pub name: String,
    pub email: String,
    pub flow: String,
    pub stats: TrafficStats,
    pub online_count: u32,
}

impl XrayUser {
    pub fn email_from_name(name: &str) -> String {
        format!("{}@vpn", name)
    }
}

/// Traffic statistics for a user
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrafficStats {
    pub uplink: u64,
    pub downlink: u64,
}

/// Parameters needed to construct a vless:// URL
#[derive(Debug, Clone)]
pub struct VlessUrlParams {
    pub uuid: String,
    pub host: String,
    pub port: u16,
    pub sni: String,
    pub public_key: String,
    pub short_id: String,
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_email_from_name() {
        assert_eq!(XrayUser::email_from_name("alice"), "alice@vpn");
        assert_eq!(XrayUser::email_from_name("bob"), "bob@vpn");
    }

    #[test]
    fn test_traffic_stats_default() {
        let stats = TrafficStats::default();
        assert_eq!(stats.uplink, 0);
        assert_eq!(stats.downlink, 0);
    }
}
