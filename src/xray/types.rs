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

/// Traffic statistics for a user
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrafficStats {
    pub uplink: u64,
    pub downlink: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_traffic_stats_default() {
        let stats = TrafficStats::default();
        assert_eq!(stats.uplink, 0);
        assert_eq!(stats.downlink, 0);
    }
}
