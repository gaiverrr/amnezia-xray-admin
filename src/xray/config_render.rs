//! Pure config.json template rendering (bridge inbound, egress inbound+outbound, routing).

use crate::error::{AppError, Result};

#[derive(Debug, Clone)]
pub struct ClientEntry {
    pub uuid: String,
    pub email: String,
}

#[derive(Debug)]
pub struct ParsedBridgeConfig {
    pub clients: Vec<ClientEntry>,
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

    Ok(ParsedBridgeConfig { clients })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bridge_config_extracts_clients() {
        let raw = include_str!("../../tests/fixtures/bridge-config-sample.json");
        let parsed = parse_bridge_config(raw).unwrap();

        assert_eq!(parsed.clients.len(), 2);
        assert_eq!(parsed.clients[0].email, "alice@vpn");
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
