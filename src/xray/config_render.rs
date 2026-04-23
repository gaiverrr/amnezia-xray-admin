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
    serde_json::to_string_pretty(&value)
        .map_err(|e| AppError::Config(format!("render egress config: {e}")))
}

#[derive(Debug)]
pub struct ParsedBridgeConfig {
    pub clients: Vec<ClientEntry>,
    pub egress_outbound: EgressOutbound,
}

pub fn parse_bridge_config(raw: &str) -> Result<ParsedBridgeConfig> {
    let v: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| AppError::Config(format!("parse bridge config: {e}")))?;

    let inbound = v["inbounds"][0].clone();
    let network = inbound["streamSettings"]["network"].as_str().unwrap_or("");
    let security = inbound["streamSettings"]["security"].as_str().unwrap_or("");
    if network != "xhttp" || security != "reality" {
        return Err(AppError::Config(format!(
            "bridge inbound must be network=xhttp security=reality, got network={network} security={security}"
        )));
    }

    let clients = inbound["settings"]["clients"]
        .as_array()
        .ok_or_else(|| AppError::Config("missing clients array".into()))?
        .iter()
        .map(|c| ClientEntry {
            uuid: c["id"].as_str().unwrap_or("").to_string(),
            email: c["email"].as_str().unwrap_or("").to_string(),
        })
        .collect();

    let outbound_raw = v["outbounds"]
        .as_array()
        .and_then(|arr| arr.iter().find(|o| o["tag"] == "foreign-egress"))
        .ok_or_else(|| AppError::Config("foreign-egress outbound not found".into()))?;

    let egress = EgressOutbound {
        address: outbound_raw["settings"]["vnext"][0]["address"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        port: outbound_raw["settings"]["vnext"][0]["port"]
            .as_u64()
            .unwrap_or(0) as u16,
        bridge_uuid: outbound_raw["settings"]["vnext"][0]["users"][0]["id"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        xhttp_path: outbound_raw["streamSettings"]["xhttpSettings"]["path"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        reality_public_key: outbound_raw["streamSettings"]["realitySettings"]["publicKey"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        reality_short_id: outbound_raw["streamSettings"]["realitySettings"]["shortId"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        server_name: outbound_raw["streamSettings"]["realitySettings"]["serverName"]
            .as_str()
            .unwrap_or("")
            .to_string(),
    };

    Ok(ParsedBridgeConfig {
        clients,
        egress_outbound: egress,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_config_matches_snapshot() {
        let input = BridgeConfigInput {
            clients: vec![
                ClientEntry {
                    uuid: "00000000-0000-0000-0000-000000000001".into(),
                    email: "alice@vpn".into(),
                },
                ClientEntry {
                    uuid: "00000000-0000-0000-0000-000000000002".into(),
                    email: "bob@vpn".into(),
                },
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
        let expected: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/bridge-config-sample.json"
        ))
        .unwrap();

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
        let expected: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/egress-config-sample.json"
        ))
        .unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn parse_bridge_config_extracts_clients_and_outbound() {
        let raw = include_str!("../../tests/fixtures/bridge-config-sample.json");
        let parsed = parse_bridge_config(raw).unwrap();

        assert_eq!(parsed.clients.len(), 2);
        assert_eq!(parsed.clients[0].email, "alice@vpn");
        assert_eq!(parsed.egress_outbound.address, "1.2.3.4");
        assert_eq!(parsed.egress_outbound.port, 8444);
        assert_eq!(parsed.egress_outbound.server_name, "example.duckdns.org");
    }

    #[test]
    fn parse_bridge_config_rejects_non_xhttp() {
        let raw = r#"{
            "inbounds": [{
                "protocol": "vless",
                "streamSettings": {"network": "tcp", "security": "reality"}
            }],
            "outbounds": []
        }"#;
        let err = parse_bridge_config(raw).unwrap_err();
        assert!(err.to_string().contains("xhttp"));
    }
}
