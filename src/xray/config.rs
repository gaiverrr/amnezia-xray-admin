// Xray server config operations
//
// The core parsing and mutation logic lives in types.rs (ServerConfig, ClientsTable).
// This module houses higher-level operations like ensure_api_enabled().

use super::types::{ClientsTable, ServerConfig};
use crate::error::{AppError, Result};
use crate::ssh::SshSession;
use serde_json::json;

pub(crate) const SERVER_CONFIG_PATH: &str = "/opt/amnezia/xray/server.json";
pub(crate) const CLIENTS_TABLE_PATH: &str = "/opt/amnezia/xray/clientsTable";
const API_LISTEN_PORT: u16 = 8080;

/// Ensure the Xray API is enabled in the server config.
///
/// This transforms server.json to add:
/// - `api` section with HandlerService and StatsService
/// - `stats` section (empty)
/// - `policy` section with stats enabled
/// - `routing` rule to direct API traffic
/// - `dokodemo-door` inbound on 127.0.0.1:8080
/// - `tag: "vless-in"` on the main VLESS inbound
/// - `email` field on each existing client
///
/// Returns true if the config was modified, false if API was already enabled.
pub fn enable_api(config: &mut ServerConfig, clients_table: &ClientsTable) -> Result<bool> {
    if config.has_api() {
        return Ok(false);
    }

    add_api_section(config);
    add_stats_section(config);
    add_policy_section(config);
    add_api_routing_rule(config);
    add_api_inbound(config);
    tag_vless_inbound(config)?;
    add_email_to_clients(config, clients_table)?;

    Ok(true)
}

fn add_api_section(config: &mut ServerConfig) {
    config.raw["api"] = json!({
        "tag": "api",
        "services": ["HandlerService", "StatsService"]
    });
}

fn add_stats_section(config: &mut ServerConfig) {
    config.raw["stats"] = json!({});
}

fn add_policy_section(config: &mut ServerConfig) {
    config.raw["policy"] = json!({
        "levels": {
            "0": {
                "statsUserUplink": true,
                "statsUserDownlink": true
            }
        },
        "system": {
            "statsInboundUplink": true,
            "statsInboundDownlink": true
        }
    });
}

fn add_api_routing_rule(config: &mut ServerConfig) {
    let api_rule = json!({
        "inboundTag": ["api"],
        "outboundTag": "api",
        "type": "field"
    });

    if let Some(routing) = config.raw.get_mut("routing") {
        if let Some(rules) = routing.get_mut("rules").and_then(|r| r.as_array_mut()) {
            // Don't add if already present
            let already_has = rules.iter().any(|r| {
                r.get("outboundTag")
                    .and_then(|t| t.as_str())
                    .map(|t| t == "api")
                    .unwrap_or(false)
            });
            if !already_has {
                rules.insert(0, api_rule);
            }
        } else {
            routing["rules"] = json!([api_rule]);
        }
    } else {
        config.raw["routing"] = json!({
            "rules": [api_rule]
        });
    }
}

fn add_api_inbound(config: &mut ServerConfig) {
    let api_inbound = json!({
        "tag": "api",
        "port": API_LISTEN_PORT,
        "listen": "127.0.0.1",
        "protocol": "dokodemo-door",
        "settings": {
            "address": "127.0.0.1"
        }
    });

    if let Some(inbounds) = config
        .raw
        .get_mut("inbounds")
        .and_then(|i| i.as_array_mut())
    {
        // Don't add if already present
        let already_has = inbounds.iter().any(|ib| {
            ib.get("tag")
                .and_then(|t| t.as_str())
                .map(|t| t == "api")
                .unwrap_or(false)
        });
        if !already_has {
            inbounds.push(api_inbound);
        }
    }
}

fn tag_vless_inbound(config: &mut ServerConfig) -> Result<()> {
    let inbound = config
        .find_vless_inbound_mut()
        .ok_or_else(|| AppError::Xray("no VLESS inbound found".into()))?;

    if inbound.get("tag").is_none() {
        inbound["tag"] = json!("vless-in");
    }

    Ok(())
}

fn add_email_to_clients(config: &mut ServerConfig, clients_table: &ClientsTable) -> Result<()> {
    let inbound = config
        .find_vless_inbound_mut()
        .ok_or_else(|| AppError::Xray("no VLESS inbound found".into()))?;

    let clients = inbound
        .get_mut("settings")
        .and_then(|s| s.get_mut("clients"))
        .and_then(|c| c.as_array_mut())
        .ok_or_else(|| AppError::Xray("no clients array found".into()))?;

    for client in clients.iter_mut() {
        if client.get("email").and_then(|e| e.as_str()).is_some() {
            continue;
        }

        let uuid = client
            .get("id")
            .and_then(|id| id.as_str())
            .unwrap_or("unknown");

        let name = clients_table.name_for_uuid(uuid).unwrap_or(uuid);

        let email = format!("{}@vpn", name);
        client["email"] = json!(email);
    }

    Ok(())
}

/// Upload the modified server config and restart the Xray container.
pub async fn upload_and_restart(
    session: &SshSession,
    config: &ServerConfig,
    container: &str,
) -> Result<()> {
    let json = config.to_json();

    // Escape single quotes in JSON for shell safety
    let escaped = json.replace('\'', "'\\''");
    let write_cmd = format!("printf '%s' '{}' > {}", escaped, SERVER_CONFIG_PATH);
    let result = session.exec_command(&write_cmd).await?;
    if !result.success() {
        return Err(AppError::Xray(format!(
            "failed to write server config: {}",
            result.stderr
        )));
    }

    // Restart the container to apply changes
    let restart_cmd = format!("docker restart {}", container);
    let result = session.exec_command(&restart_cmd).await?;
    if !result.success() {
        return Err(AppError::Xray(format!(
            "failed to restart container: {}",
            result.stderr
        )));
    }

    Ok(())
}

/// Read server.json from the remote server.
pub async fn read_server_config(session: &SshSession) -> Result<ServerConfig> {
    let cmd = format!("cat {}", SERVER_CONFIG_PATH);
    let result = session.exec_command(&cmd).await?;
    if !result.success() {
        return Err(AppError::Xray(format!(
            "failed to read server config: {}",
            result.stderr
        )));
    }
    ServerConfig::parse(&result.stdout)
}

/// Read clientsTable from the remote server.
pub async fn read_clients_table(session: &SshSession) -> Result<ClientsTable> {
    let cmd = format!("cat {}", CLIENTS_TABLE_PATH);
    let result = session.exec_command(&cmd).await?;
    if !result.success() {
        return Err(AppError::Xray(format!(
            "failed to read clients table: {}",
            result.stderr
        )));
    }
    ClientsTable::parse(&result.stdout)
}

/// High-level: ensure API is enabled on the remote server.
/// Reads config, transforms it if needed, uploads and restarts.
/// Returns true if changes were made, false if API was already enabled.
pub async fn ensure_api_enabled(session: &SshSession, container: &str) -> Result<bool> {
    let mut config = read_server_config(session).await?;
    let clients_table = read_clients_table(session).await?;

    let modified = enable_api(&mut config, &clients_table)?;
    if modified {
        upload_and_restart(session, &config, container).await?;
    }

    Ok(modified)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xray::types::ClientsTable;

    fn sample_server_json() -> &'static str {
        r#"{
            "inbounds": [
                {
                    "listen": "0.0.0.0",
                    "port": 443,
                    "protocol": "vless",
                    "settings": {
                        "clients": [
                            {
                                "id": "aaaaaaaa-1111-2222-3333-bbbbbbbbbbbb",
                                "flow": "xtls-rprx-vision"
                            },
                            {
                                "id": "cccccccc-4444-5555-6666-dddddddddddd",
                                "flow": "xtls-rprx-vision",
                                "email": "alice@vpn"
                            }
                        ],
                        "decryption": "none"
                    },
                    "streamSettings": {
                        "network": "tcp",
                        "security": "reality",
                        "realitySettings": {
                            "dest": "www.googletagmanager.com:443",
                            "serverNames": ["www.googletagmanager.com"],
                            "privateKey": "test-private-key",
                            "shortIds": ["abcd1234"]
                        }
                    }
                }
            ],
            "outbounds": [
                {"protocol": "freedom", "tag": "direct"}
            ]
        }"#
    }

    fn sample_clients_table() -> &'static str {
        r#"[
            {"clientId": "aaaaaaaa-1111-2222-3333-bbbbbbbbbbbb", "userData": "bob"},
            {"clientId": "cccccccc-4444-5555-6666-dddddddddddd", "userData": "alice"}
        ]"#
    }

    fn sample_server_json_with_api() -> &'static str {
        r#"{
            "api": {"tag": "api", "services": ["HandlerService", "StatsService"]},
            "stats": {},
            "policy": {
                "levels": {"0": {"statsUserUplink": true, "statsUserDownlink": true}},
                "system": {"statsInboundUplink": true, "statsInboundDownlink": true}
            },
            "routing": {
                "rules": [{"inboundTag": ["api"], "outboundTag": "api", "type": "field"}]
            },
            "inbounds": [
                {
                    "tag": "api",
                    "port": 8080,
                    "listen": "127.0.0.1",
                    "protocol": "dokodemo-door",
                    "settings": {"address": "127.0.0.1"}
                },
                {
                    "tag": "vless-in",
                    "listen": "0.0.0.0",
                    "port": 443,
                    "protocol": "vless",
                    "settings": {
                        "clients": [
                            {"id": "uuid1", "flow": "xtls-rprx-vision", "email": "bob@vpn"}
                        ],
                        "decryption": "none"
                    }
                }
            ],
            "outbounds": [{"protocol": "freedom", "tag": "direct"}]
        }"#
    }

    #[test]
    fn test_enable_api_transforms_config() {
        let mut config = ServerConfig::parse(sample_server_json()).unwrap();
        let table = ClientsTable::parse(sample_clients_table()).unwrap();

        assert!(!config.has_api());
        let modified = enable_api(&mut config, &table).unwrap();
        assert!(modified);
        assert!(config.has_api());

        // Verify api section
        let api = config.raw.get("api").unwrap();
        assert_eq!(api["tag"], "api");
        let services = api["services"].as_array().unwrap();
        assert!(services.iter().any(|s| s == "HandlerService"));
        assert!(services.iter().any(|s| s == "StatsService"));
    }

    #[test]
    fn test_enable_api_adds_stats_section() {
        let mut config = ServerConfig::parse(sample_server_json()).unwrap();
        let table = ClientsTable::parse(sample_clients_table()).unwrap();

        enable_api(&mut config, &table).unwrap();
        assert!(config.raw.get("stats").is_some());
    }

    #[test]
    fn test_enable_api_adds_policy_section() {
        let mut config = ServerConfig::parse(sample_server_json()).unwrap();
        let table = ClientsTable::parse(sample_clients_table()).unwrap();

        enable_api(&mut config, &table).unwrap();

        let policy = config.raw.get("policy").unwrap();
        assert_eq!(policy["levels"]["0"]["statsUserUplink"], true);
        assert_eq!(policy["levels"]["0"]["statsUserDownlink"], true);
        assert_eq!(policy["system"]["statsInboundUplink"], true);
        assert_eq!(policy["system"]["statsInboundDownlink"], true);
    }

    #[test]
    fn test_enable_api_adds_routing_rule() {
        let mut config = ServerConfig::parse(sample_server_json()).unwrap();
        let table = ClientsTable::parse(sample_clients_table()).unwrap();

        enable_api(&mut config, &table).unwrap();

        let routing = config.raw.get("routing").unwrap();
        let rules = routing["rules"].as_array().unwrap();
        assert!(!rules.is_empty());
        let api_rule = &rules[0];
        assert_eq!(api_rule["outboundTag"], "api");
        let inbound_tags = api_rule["inboundTag"].as_array().unwrap();
        assert!(inbound_tags.iter().any(|t| t == "api"));
    }

    #[test]
    fn test_enable_api_adds_dokodemo_door_inbound() {
        let mut config = ServerConfig::parse(sample_server_json()).unwrap();
        let table = ClientsTable::parse(sample_clients_table()).unwrap();

        enable_api(&mut config, &table).unwrap();

        let inbounds = config.raw["inbounds"].as_array().unwrap();
        let api_inbound = inbounds
            .iter()
            .find(|ib| {
                ib.get("tag")
                    .and_then(|t| t.as_str())
                    .map(|t| t == "api")
                    .unwrap_or(false)
            })
            .expect("API inbound should exist");

        assert_eq!(api_inbound["port"], API_LISTEN_PORT);
        assert_eq!(api_inbound["listen"], "127.0.0.1");
        assert_eq!(api_inbound["protocol"], "dokodemo-door");
    }

    #[test]
    fn test_enable_api_tags_vless_inbound() {
        let mut config = ServerConfig::parse(sample_server_json()).unwrap();
        let table = ClientsTable::parse(sample_clients_table()).unwrap();

        enable_api(&mut config, &table).unwrap();

        let inbounds = config.raw["inbounds"].as_array().unwrap();
        let vless_inbound = inbounds
            .iter()
            .find(|ib| {
                ib.get("protocol")
                    .and_then(|p| p.as_str())
                    .map(|p| p == "vless")
                    .unwrap_or(false)
            })
            .expect("VLESS inbound should exist");

        assert_eq!(vless_inbound["tag"], "vless-in");
    }

    #[test]
    fn test_enable_api_adds_email_to_clients() {
        let mut config = ServerConfig::parse(sample_server_json()).unwrap();
        let table = ClientsTable::parse(sample_clients_table()).unwrap();

        enable_api(&mut config, &table).unwrap();

        let clients = config.clients();

        // bob: had no email, should now have bob@vpn (from clientsTable name)
        let bob = clients
            .iter()
            .find(|c| c.id == "aaaaaaaa-1111-2222-3333-bbbbbbbbbbbb")
            .unwrap();
        assert_eq!(bob.email, Some("bob@vpn".to_string()));

        // alice: already had alice@vpn, should be unchanged
        let alice = clients
            .iter()
            .find(|c| c.id == "cccccccc-4444-5555-6666-dddddddddddd")
            .unwrap();
        assert_eq!(alice.email, Some("alice@vpn".to_string()));
    }

    #[test]
    fn test_enable_api_skips_if_already_enabled() {
        let mut config = ServerConfig::parse(sample_server_json_with_api()).unwrap();
        let table = ClientsTable::parse(sample_clients_table()).unwrap();

        let modified = enable_api(&mut config, &table).unwrap();
        assert!(!modified);
    }

    #[test]
    fn test_enable_api_preserves_existing_fields() {
        let mut config = ServerConfig::parse(sample_server_json()).unwrap();
        let table = ClientsTable::parse(sample_clients_table()).unwrap();

        enable_api(&mut config, &table).unwrap();

        // Check that existing outbounds are preserved
        let outbounds = config.raw["outbounds"].as_array().unwrap();
        assert_eq!(outbounds.len(), 1);
        assert_eq!(outbounds[0]["protocol"], "freedom");

        // Check that VLESS inbound still has its stream settings
        let vless = config.raw["inbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|ib| ib["protocol"] == "vless")
            .unwrap();
        assert_eq!(vless["streamSettings"]["security"], "reality");
        assert_eq!(vless["port"], 443);
    }

    #[test]
    fn test_enable_api_client_without_table_entry_uses_uuid() {
        let json = r#"{
            "inbounds": [{
                "protocol": "vless",
                "settings": {
                    "clients": [
                        {"id": "unknown-uuid", "flow": "xtls-rprx-vision"}
                    ]
                }
            }]
        }"#;
        let mut config = ServerConfig::parse(json).unwrap();
        let table = ClientsTable::parse("[]").unwrap();

        enable_api(&mut config, &table).unwrap();

        let clients = config.clients();
        assert_eq!(clients[0].email, Some("unknown-uuid@vpn".to_string()));
    }

    #[test]
    fn test_enable_api_existing_routing_preserved() {
        let json = r#"{
            "inbounds": [{
                "protocol": "vless",
                "settings": {
                    "clients": [{"id": "uuid1", "flow": "xtls-rprx-vision"}]
                }
            }],
            "routing": {
                "rules": [
                    {"type": "field", "outboundTag": "block", "domain": ["geosite:category-ads"]}
                ]
            }
        }"#;
        let mut config = ServerConfig::parse(json).unwrap();
        let table = ClientsTable::parse("[]").unwrap();

        enable_api(&mut config, &table).unwrap();

        let rules = config.raw["routing"]["rules"].as_array().unwrap();
        // API rule should be first, existing rule preserved
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0]["outboundTag"], "api");
        assert_eq!(rules[1]["outboundTag"], "block");
    }

    #[test]
    fn test_enable_api_preserves_existing_tag() {
        let json = r#"{
            "inbounds": [{
                "protocol": "vless",
                "tag": "my-custom-tag",
                "settings": {
                    "clients": [{"id": "uuid1", "flow": "xtls-rprx-vision"}]
                }
            }]
        }"#;
        let mut config = ServerConfig::parse(json).unwrap();
        let table = ClientsTable::parse("[]").unwrap();

        enable_api(&mut config, &table).unwrap();

        let vless = config.raw["inbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|ib| ib["protocol"] == "vless")
            .unwrap();
        // Should keep existing tag, not overwrite
        assert_eq!(vless["tag"], "my-custom-tag");
    }

    #[test]
    fn test_enable_api_roundtrip_json() {
        let mut config = ServerConfig::parse(sample_server_json()).unwrap();
        let table = ClientsTable::parse(sample_clients_table()).unwrap();

        enable_api(&mut config, &table).unwrap();

        // Roundtrip through JSON
        let json = config.to_json();
        let config2 = ServerConfig::parse(&json).unwrap();
        assert!(config2.has_api());
        assert_eq!(config2.clients().len(), 2);
    }

    #[test]
    fn test_enable_api_no_vless_inbound_errors() {
        let json = r#"{
            "inbounds": [{"protocol": "vmess", "settings": {"clients": []}}]
        }"#;
        let mut config = ServerConfig::parse(json).unwrap();
        let table = ClientsTable::parse("[]").unwrap();

        let result = enable_api(&mut config, &table);
        assert!(result.is_err());
    }
}
