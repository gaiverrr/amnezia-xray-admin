//! Pure config.json template rendering (bridge inbound, egress inbound+outbound, routing).

use crate::error::{AppError, Result};

#[derive(Debug, Clone)]
pub struct ClientEntry {
    pub uuid: String,
    pub email: String,
}

#[derive(Debug, Clone)]
pub struct EgressOutbound {
    pub address: String,
    pub port: u16,
    pub bridge_uuid: String,
    pub xhttp_path: String,
    pub reality_public_key: String,
    pub reality_short_id: String,
    pub server_name: String,
}

#[derive(Debug, Clone)]
pub struct BridgeConfigInput {
    pub clients: Vec<ClientEntry>,
    pub reality_private_key: String,
    pub reality_short_id: String,
    pub xhttp_path: String,
    pub sni: String,
    pub egress_outbound: EgressOutbound,
}

pub fn render_bridge_config(input: &BridgeConfigInput) -> Result<String> {
    let value = serde_json::json!({
        "log": {
            "loglevel": "warning",
            "access": "/var/log/xray/access.log",
            "error": "/var/log/xray/error.log"
        },
        "inbounds": [{
            "listen": "0.0.0.0",
            "port": 443,
            "tag": "client-in",
            "protocol": "vless",
            "settings": {
                "clients": input.clients.iter().map(|c| serde_json::json!({
                    "id": c.uuid,
                    "email": c.email
                })).collect::<Vec<_>>(),
                "decryption": "none"
            },
            "streamSettings": {
                "network": "xhttp",
                "security": "reality",
                "xhttpSettings": {"path": input.xhttp_path},
                "realitySettings": {
                    "dest": format!("{}:443", input.sni),
                    "serverNames": [input.sni],
                    "privateKey": input.reality_private_key,
                    "shortIds": [input.reality_short_id]
                }
            }
        }],
        "outbounds": [
            {
                "tag": "foreign-egress",
                "protocol": "vless",
                "settings": {
                    "vnext": [{
                        "address": input.egress_outbound.address,
                        "port": input.egress_outbound.port,
                        "users": [{
                            "id": input.egress_outbound.bridge_uuid,
                            "encryption": "none"
                        }]
                    }]
                },
                "streamSettings": {
                    "network": "xhttp",
                    "security": "reality",
                    "xhttpSettings": {"path": input.egress_outbound.xhttp_path},
                    "realitySettings": {
                        "fingerprint": "chrome",
                        "serverName": input.egress_outbound.server_name,
                        "publicKey": input.egress_outbound.reality_public_key,
                        "shortId": input.egress_outbound.reality_short_id
                    }
                }
            },
            {"protocol": "freedom", "tag": "direct"},
            {"protocol": "blackhole", "tag": "block"}
        ],
        "routing": {
            "domainStrategy": "IPIfNonMatch",
            "rules": [
                {"type": "field", "ip": ["geoip:private"], "outboundTag": "direct"},
                {"type": "field", "domain": ["geosite:category-ru"], "outboundTag": "direct"},
                {"type": "field", "ip": ["geoip:ru"], "outboundTag": "direct"},
                {"type": "field", "network": "tcp,udp", "outboundTag": "foreign-egress"}
            ]
        }
    });
    serde_json::to_string_pretty(&value)
        .map_err(|e| AppError::Config(format!("render bridge config: {e}")))
}

#[derive(Debug, Clone)]
pub struct EgressConfigInput {
    pub bridge_uuid: String,
    pub port: u16,
    pub xhttp_path: String,
    pub reality_private_key: String,
    pub reality_short_id: String,
    pub domain: String,
    pub nginx_port: u16,
}

pub fn render_egress_config(input: &EgressConfigInput) -> Result<String> {
    let value = serde_json::json!({
        "log": {
            "loglevel": "warning",
            "access": "/var/log/xray/access.log",
            "error": "/var/log/xray/error.log"
        },
        "inbounds": [{
            "listen": "0.0.0.0",
            "port": input.port,
            "tag": "bridge-in",
            "protocol": "vless",
            "settings": {
                "clients": [{
                    "id": input.bridge_uuid,
                    "email": "bridge@vpn"
                }],
                "decryption": "none"
            },
            "streamSettings": {
                "network": "xhttp",
                "security": "reality",
                "xhttpSettings": {"path": input.xhttp_path},
                "realitySettings": {
                    "dest": format!("127.0.0.1:{}", input.nginx_port),
                    "serverNames": [input.domain],
                    "privateKey": input.reality_private_key,
                    "shortIds": [input.reality_short_id]
                }
            }
        }],
        "outbounds": [{"protocol": "freedom", "tag": "direct"}]
    });
    serde_json::to_string_pretty(&value).map_err(|e| AppError::Config(format!("render egress config: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_config_matches_snapshot() {
        let input = BridgeConfigInput {
            clients: vec![
                ClientEntry { uuid: "00000000-0000-0000-0000-000000000001".into(), email: "alice@vpn".into() },
                ClientEntry { uuid: "00000000-0000-0000-0000-000000000002".into(), email: "bob@vpn".into() },
            ],
            reality_private_key: "TEST_PRIVATE_KEY".into(),
            reality_short_id: "TESTSID".into(),
            xhttp_path: "/testpath".into(),
            sni: "www.sberbank.ru".into(),
            egress_outbound: EgressOutbound {
                address: "1.2.3.4".into(),
                port: 8444,
                bridge_uuid: "00000000-0000-0000-0000-000000000009".into(),
                xhttp_path: "/egresspath".into(),
                reality_public_key: "EGRESS_PUBLIC".into(),
                reality_short_id: "EGRESSSID".into(),
                server_name: "example.duckdns.org".into(),
            },
        };

        let rendered = render_bridge_config(&input).unwrap();
        let actual: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        let expected: serde_json::Value = serde_json::from_str(
            include_str!("../../tests/fixtures/bridge-config-sample.json")
        ).unwrap();

        assert_eq!(actual, expected);
    }

    #[test]
    fn egress_config_matches_snapshot() {
        let input = EgressConfigInput {
            bridge_uuid: "00000000-0000-0000-0000-000000000009".into(),
            port: 8444,
            xhttp_path: "/egresspath".into(),
            reality_private_key: "EGRESS_PRIVATE".into(),
            reality_short_id: "EGRESSSID".into(),
            domain: "example.duckdns.org".into(),
            nginx_port: 9443,
        };
        let rendered = render_egress_config(&input).unwrap();
        let actual: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        let expected: serde_json::Value = serde_json::from_str(
            include_str!("../../tests/fixtures/egress-config-sample.json")
        ).unwrap();
        assert_eq!(actual, expected);
    }
}
