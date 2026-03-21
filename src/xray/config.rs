// Xray server config operations
//
// The core parsing and mutation logic lives in types.rs (ServerConfig, ClientsTable).
// This module houses higher-level operations like ensure_api_enabled().

use super::types::{ClientsTable, ServerConfig};
use crate::backend_trait::XrayBackend;
use crate::error::{AppError, Result};
use base64::Engine;
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
    // Check and add each required section independently, so that a partial
    // config (e.g. `api` present but missing stats/policy/routing/inbound)
    // gets fully repaired.
    let mut added_any_section = false;

    if !config.has_api() {
        add_api_section(config);
        added_any_section = true;
    }
    if !config.raw.get("stats").is_some_and(|v| v.is_object()) {
        add_stats_section(config);
        added_any_section = true;
    }
    if !has_complete_policy(config) {
        add_policy_section(config);
        added_any_section = true;
    }
    let added_routing = add_api_routing_rule(config);
    let added_inbound = add_api_inbound(config);
    let added_api = added_any_section || added_routing || added_inbound;

    // Always normalize the VLESS inbound tag and client emails, even when the
    // API section already exists.  A partially configured server (e.g. manual
    // `api` key but missing `vless-in` tag or client emails) would otherwise
    // silently break add/delete/stats operations that rely on these values.
    let tag_changed = normalize_vless_tag(config)?;
    let emails_changed = add_email_to_clients(config, clients_table)?;

    Ok(added_api || tag_changed || emails_changed)
}

fn add_api_section(config: &mut ServerConfig) {
    // Merge into existing api object instead of replacing, so that
    // user-configured services (e.g. LoggerService, RoutingService) are
    // preserved.
    if !config.raw.get("api").is_some_and(|v| v.is_object()) {
        config.raw["api"] = json!({});
    }
    let api = config.raw["api"].as_object_mut().unwrap();
    api.insert("tag".to_string(), json!("api"));

    // Ensure required services are present, keeping any extras
    if !api.get("services").is_some_and(|v| v.is_array()) {
        api.insert("services".to_string(), json!([]));
    }
    let services = api.get_mut("services").unwrap().as_array_mut().unwrap();
    for required in &["HandlerService", "StatsService"] {
        if !services.iter().any(|s| s.as_str() == Some(required)) {
            services.push(json!(required));
        }
    }
}

fn add_stats_section(config: &mut ServerConfig) {
    config.raw["stats"] = json!({});
}

/// Check whether the policy section has all required stats flags set to true.
fn has_complete_policy(config: &ServerConfig) -> bool {
    let Some(policy) = config.raw.get("policy") else {
        return false;
    };
    let has_user = policy
        .get("levels")
        .and_then(|l| l.get("0"))
        .map(|l0| {
            l0.get("statsUserUplink").and_then(|v| v.as_bool()) == Some(true)
                && l0.get("statsUserDownlink").and_then(|v| v.as_bool()) == Some(true)
        })
        .unwrap_or(false);
    let has_system = policy
        .get("system")
        .map(|s| {
            s.get("statsInboundUplink").and_then(|v| v.as_bool()) == Some(true)
                && s.get("statsInboundDownlink").and_then(|v| v.as_bool()) == Some(true)
        })
        .unwrap_or(false);
    has_user && has_system
}

/// Merge required stats flags into the policy section, preserving any
/// existing keys (e.g. custom level rules) that are not ours to touch.
fn add_policy_section(config: &mut ServerConfig) {
    // Reset policy to an object if it's missing or not an object (e.g. null)
    if !config.raw.get("policy").is_some_and(|v| v.is_object()) {
        config.raw["policy"] = json!({});
    }
    let policy = config.raw["policy"].as_object_mut().unwrap();

    // Ensure levels.0 has stats flags, but keep other levels/fields intact
    if !policy.get("levels").is_some_and(|v| v.is_object()) {
        policy.insert("levels".to_string(), json!({}));
    }
    let levels = policy.get_mut("levels").unwrap().as_object_mut().unwrap();
    if !levels.get("0").is_some_and(|v| v.is_object()) {
        levels.insert("0".to_string(), json!({}));
    }
    let level0 = levels.get_mut("0").unwrap();
    level0["statsUserUplink"] = json!(true);
    level0["statsUserDownlink"] = json!(true);

    // Ensure system has stats flags, but keep other system fields intact
    if !policy.get("system").is_some_and(|v| v.is_object()) {
        policy.insert("system".to_string(), json!({}));
    }
    let system = policy.get_mut("system").unwrap();
    system["statsInboundUplink"] = json!(true);
    system["statsInboundDownlink"] = json!(true);
}

/// Check whether a routing rule is a correct API routing rule.
fn is_valid_api_routing_rule(rule: &serde_json::Value) -> bool {
    let has_outbound = rule.get("outboundTag").and_then(|t| t.as_str()) == Some("api");
    let has_inbound = rule
        .get("inboundTag")
        .and_then(|t| t.as_array())
        .map(|tags| tags.iter().any(|t| t.as_str() == Some("api")))
        .unwrap_or(false);
    let has_type = rule.get("type").and_then(|t| t.as_str()) == Some("field");
    has_outbound && has_inbound && has_type
}

fn add_api_routing_rule(config: &mut ServerConfig) -> bool {
    let api_rule = json!({
        "inboundTag": ["api"],
        "outboundTag": "api",
        "type": "field"
    });

    if let Some(routing) = config.raw.get_mut("routing") {
        if let Some(rules) = routing.get_mut("rules").and_then(|r| r.as_array_mut()) {
            // Find any existing rule with outboundTag "api"
            if let Some(pos) = rules
                .iter()
                .position(|r| r.get("outboundTag").and_then(|t| t.as_str()) == Some("api"))
            {
                if is_valid_api_routing_rule(&rules[pos]) {
                    return false;
                }
                // Replace the broken rule in-place
                rules[pos] = api_rule;
                return true;
            }
            rules.insert(0, api_rule);
        } else {
            routing["rules"] = json!([api_rule]);
        }
    } else {
        config.raw["routing"] = json!({
            "rules": [api_rule]
        });
    }
    true
}

/// Check whether an inbound is a correctly configured API inbound.
fn is_valid_api_inbound(ib: &serde_json::Value) -> bool {
    let has_tag = ib.get("tag").and_then(|t| t.as_str()) == Some("api");
    let has_protocol = ib.get("protocol").and_then(|p| p.as_str()) == Some("dokodemo-door");
    let has_listen = ib.get("listen").and_then(|l| l.as_str()) == Some("127.0.0.1");
    let has_port = ib.get("port").and_then(|p| p.as_u64()) == Some(API_LISTEN_PORT as u64);
    has_tag && has_protocol && has_listen && has_port
}

fn add_api_inbound(config: &mut ServerConfig) -> bool {
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
        // Find any existing inbound tagged "api"
        if let Some(pos) = inbounds
            .iter()
            .position(|ib| ib.get("tag").and_then(|t| t.as_str()) == Some("api"))
        {
            if is_valid_api_inbound(&inbounds[pos]) {
                return false;
            }
            // Replace the broken inbound in-place
            inbounds[pos] = api_inbound;
            return true;
        }
        inbounds.push(api_inbound);
        return true;
    }
    false
}

/// Ensure the VLESS inbound has tag "vless-in". Returns true if it was changed.
/// Also updates any routing rules that reference the old tag so they keep working.
fn normalize_vless_tag(config: &mut ServerConfig) -> Result<bool> {
    let inbound = config
        .find_vless_inbound_mut()
        .ok_or_else(|| AppError::Xray("no VLESS inbound found".into()))?;

    let current_tag = inbound.get("tag").and_then(|t| t.as_str()).unwrap_or("");

    if current_tag == "vless-in" {
        return Ok(false);
    }

    let old_tag = current_tag.to_string();
    inbound["tag"] = json!("vless-in");

    // Update routing rules that reference the old tag so they don't break.
    if !old_tag.is_empty() {
        update_routing_inbound_tags(config, &old_tag, "vless-in");
    }

    Ok(true)
}

/// Replace occurrences of `old_tag` with `new_tag` inside routing rule `inboundTag` arrays.
fn update_routing_inbound_tags(config: &mut ServerConfig, old_tag: &str, new_tag: &str) {
    let rules = config
        .raw
        .get_mut("routing")
        .and_then(|r| r.get_mut("rules"))
        .and_then(|r| r.as_array_mut());

    if let Some(rules) = rules {
        for rule in rules.iter_mut() {
            if let Some(tags) = rule.get_mut("inboundTag").and_then(|t| t.as_array_mut()) {
                for tag in tags.iter_mut() {
                    if tag.as_str() == Some(old_tag) {
                        *tag = json!(new_tag);
                    }
                }
            }
        }
    }
}

/// Add email and level fields to clients that are missing them.
/// Returns true if any client was modified.
fn add_email_to_clients(config: &mut ServerConfig, clients_table: &ClientsTable) -> Result<bool> {
    let inbound = config
        .find_vless_inbound_mut()
        .ok_or_else(|| AppError::Xray("no VLESS inbound found".into()))?;

    let clients = inbound
        .get_mut("settings")
        .and_then(|s| s.get_mut("clients"))
        .and_then(|c| c.as_array_mut())
        .ok_or_else(|| AppError::Xray("no clients array found".into()))?;

    let mut changed = false;
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
        client["level"] = json!(0);
        changed = true;
    }

    Ok(changed)
}

/// Upload the modified server config and restart the Xray container.
pub async fn upload_and_restart(backend: &dyn XrayBackend, config: &ServerConfig) -> Result<()> {
    let json = config.to_json();

    // Use base64 encoding to safely transfer JSON over shell
    let b64 = base64::engine::general_purpose::STANDARD.encode(json.as_bytes());
    let tmp = format!("{}.tmp", SERVER_CONFIG_PATH);
    let write_cmd = format!(
        "sh -c 'echo {} | base64 -d > {} && mv {} {}'",
        b64, tmp, tmp, SERVER_CONFIG_PATH
    );
    let result = backend.exec_in_container(&write_cmd).await?;
    if !result.success() {
        return Err(AppError::Xray(format!(
            "failed to write server config: {}",
            result.stderr
        )));
    }

    // Restart the container to apply changes (must run on host)
    let restart_cmd = format!("docker restart {}", backend.container_name());
    let result = backend.exec_on_host(&restart_cmd).await?;
    if !result.success() {
        return Err(AppError::Xray(format!(
            "failed to restart container: {}",
            result.stderr
        )));
    }

    Ok(())
}

/// Read server.json from the remote server.
pub async fn read_server_config(backend: &dyn XrayBackend) -> Result<ServerConfig> {
    let cmd = format!("cat {}", SERVER_CONFIG_PATH);
    let result = backend.exec_in_container(&cmd).await?;
    if !result.success() {
        return Err(AppError::Xray(format!(
            "failed to read server config: {}",
            result.stderr
        )));
    }
    ServerConfig::parse(&result.stdout)
}

/// Read clientsTable from the remote server.
pub async fn read_clients_table(backend: &dyn XrayBackend) -> Result<ClientsTable> {
    let cmd = format!("cat {}", CLIENTS_TABLE_PATH);
    let result = backend.exec_in_container(&cmd).await?;
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
pub async fn ensure_api_enabled(backend: &dyn XrayBackend) -> Result<bool> {
    let mut config = read_server_config(backend).await?;
    let clients_table = read_clients_table(backend).await?;

    let modified = enable_api(&mut config, &clients_table)?;
    if modified {
        upload_and_restart(backend, &config).await?;
        // Wait for the Xray process inside the container to become ready
        // after restart, so subsequent API calls don't fail.
        wait_for_xray_ready(backend).await?;
    }

    Ok(modified)
}

/// Poll `xray version` inside the container until it succeeds or timeout.
async fn wait_for_xray_ready(backend: &dyn XrayBackend) -> Result<()> {
    for _ in 0..10 {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if let Ok(result) = backend.exec_in_container("xray version").await {
            if result.success() {
                return Ok(());
            }
        }
    }
    Err(AppError::Xray(
        "container did not become ready after restart".to_string(),
    ))
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
            {"clientId": "aaaaaaaa-1111-2222-3333-bbbbbbbbbbbb", "userData": {"clientName": "bob", "creationDate": ""}},
            {"clientId": "cccccccc-4444-5555-6666-dddddddddddd", "userData": {"clientName": "alice", "creationDate": ""}}
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
        // Should normalize tag to "vless-in" for consistency with API commands
        assert_eq!(vless["tag"], "vless-in");
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

    #[test]
    fn test_enable_api_normalizes_partial_config() {
        // Server has API section but missing vless-in tag and client emails
        let json = r#"{
            "api": {"tag": "api", "services": ["HandlerService", "StatsService"]},
            "stats": {},
            "policy": {"levels": {"0": {"statsUserUplink": true, "statsUserDownlink": true}}},
            "inbounds": [{
                "protocol": "vless",
                "tag": "custom-tag",
                "settings": {
                    "clients": [
                        {"id": "uuid-1", "flow": "xtls-rprx-vision"}
                    ]
                }
            }]
        }"#;
        let mut config = ServerConfig::parse(json).unwrap();
        let table = ClientsTable::parse(
            r#"[{"clientId": "uuid-1", "userData": {"clientName": "alice", "creationDate": ""}}]"#,
        )
        .unwrap();

        // API already exists, but tag and emails need fixing
        assert!(config.has_api());
        let modified = enable_api(&mut config, &table).unwrap();
        assert!(
            modified,
            "should report modified when tag/emails were normalized"
        );

        // Verify tag was normalized
        let vless = config.raw["inbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|ib| ib["protocol"] == "vless")
            .unwrap();
        assert_eq!(vless["tag"], "vless-in");

        // Verify email was added
        let clients = config.clients();
        assert_eq!(clients[0].email, Some("alice@vpn".to_string()));
    }

    #[test]
    fn test_enable_api_already_normalized_returns_false() {
        // Fully configured server — nothing to change
        let mut config = ServerConfig::parse(sample_server_json_with_api()).unwrap();
        let table = ClientsTable::parse(sample_clients_table()).unwrap();

        let modified = enable_api(&mut config, &table).unwrap();
        assert!(!modified, "fully configured server should not be modified");
    }

    #[test]
    fn test_enable_api_repairs_missing_sections_when_api_exists() {
        // Server has api key but is missing stats, policy, routing rule, and api inbound
        let json = r#"{
            "api": {"tag": "api", "services": ["HandlerService", "StatsService"]},
            "inbounds": [{
                "protocol": "vless",
                "tag": "vless-in",
                "settings": {
                    "clients": [
                        {"id": "uuid-1", "flow": "xtls-rprx-vision", "email": "alice@vpn", "level": 0}
                    ],
                    "decryption": "none"
                }
            }],
            "outbounds": [{"protocol": "freedom", "tag": "direct"}]
        }"#;
        let mut config = ServerConfig::parse(json).unwrap();
        let table = ClientsTable::parse(
            r#"[{"clientId": "uuid-1", "userData": {"clientName": "alice", "creationDate": ""}}]"#,
        )
        .unwrap();

        assert!(config.has_api());
        let modified = enable_api(&mut config, &table).unwrap();
        assert!(
            modified,
            "should add missing stats/policy/routing/inbound sections"
        );

        // Verify stats was added
        assert!(config.raw.get("stats").is_some(), "stats section missing");

        // Verify policy was added
        let policy = config.raw.get("policy").expect("policy section missing");
        assert_eq!(policy["levels"]["0"]["statsUserUplink"], true);

        // Verify routing rule was added
        let rules = config.raw["routing"]["rules"].as_array().unwrap();
        assert!(
            rules.iter().any(|r| r["outboundTag"] == "api"),
            "api routing rule missing"
        );

        // Verify api inbound was added
        let inbounds = config.raw["inbounds"].as_array().unwrap();
        assert!(
            inbounds
                .iter()
                .any(|ib| ib.get("tag").and_then(|t| t.as_str()) == Some("api")),
            "api inbound missing"
        );
    }

    #[test]
    fn test_enable_api_repairs_missing_routing_rule_only() {
        // Server has everything except the routing rule for api — this was the
        // bug where add_api_routing_rule's mutation wasn't tracked in `modified`.
        let json = r#"{
            "api": {"tag": "api", "services": ["HandlerService", "StatsService"]},
            "stats": {},
            "policy": {"levels": {"0": {"statsUserUplink": true, "statsUserDownlink": true}},
                       "system": {"statsInboundUplink": true, "statsInboundDownlink": true}},
            "routing": {
                "rules": [{"type": "field", "outboundTag": "block", "domain": ["geosite:category-ads"]}]
            },
            "inbounds": [
                {"tag": "api", "port": 8080, "listen": "127.0.0.1", "protocol": "dokodemo-door",
                 "settings": {"address": "127.0.0.1"}},
                {"tag": "vless-in", "protocol": "vless",
                 "settings": {"clients": [{"id": "uuid-1", "flow": "xtls-rprx-vision", "email": "alice@vpn", "level": 0}],
                              "decryption": "none"}}
            ],
            "outbounds": [{"protocol": "freedom", "tag": "direct"}]
        }"#;
        let mut config = ServerConfig::parse(json).unwrap();
        let table = ClientsTable::parse(
            r#"[{"clientId": "uuid-1", "userData": {"clientName": "alice", "creationDate": ""}}]"#,
        )
        .unwrap();

        assert!(config.has_api());
        let modified = enable_api(&mut config, &table).unwrap();
        assert!(
            modified,
            "should report modified when only routing rule was missing"
        );

        // Verify routing rule was added
        let rules = config.raw["routing"]["rules"].as_array().unwrap();
        assert_eq!(rules[0]["outboundTag"], "api", "api rule should be first");
        assert_eq!(rules.len(), 2, "existing rule should be preserved");
    }

    #[test]
    fn test_enable_api_repairs_incomplete_policy() {
        // Server has policy but missing system stats flags
        let json = r#"{
            "api": {"tag": "api", "services": ["HandlerService", "StatsService"]},
            "stats": {},
            "policy": {
                "levels": {"0": {"statsUserUplink": true, "statsUserDownlink": true}}
            },
            "routing": {
                "rules": [{"inboundTag": ["api"], "outboundTag": "api", "type": "field"}]
            },
            "inbounds": [
                {"tag": "api", "port": 8080, "listen": "127.0.0.1", "protocol": "dokodemo-door",
                 "settings": {"address": "127.0.0.1"}},
                {"tag": "vless-in", "protocol": "vless",
                 "settings": {"clients": [{"id": "uuid-1", "flow": "xtls-rprx-vision", "email": "alice@vpn", "level": 0}],
                              "decryption": "none"}}
            ],
            "outbounds": [{"protocol": "freedom", "tag": "direct"}]
        }"#;
        let mut config = ServerConfig::parse(json).unwrap();
        let table = ClientsTable::parse(
            r#"[{"clientId": "uuid-1", "userData": {"clientName": "alice", "creationDate": ""}}]"#,
        )
        .unwrap();

        let modified = enable_api(&mut config, &table).unwrap();
        assert!(
            modified,
            "should repair incomplete policy (missing system stats)"
        );

        let policy = config.raw.get("policy").unwrap();
        assert_eq!(policy["system"]["statsInboundUplink"], true);
        assert_eq!(policy["system"]["statsInboundDownlink"], true);
        assert_eq!(policy["levels"]["0"]["statsUserUplink"], true);
        assert_eq!(policy["levels"]["0"]["statsUserDownlink"], true);
    }

    #[test]
    fn test_enable_api_policy_merge_preserves_custom_levels() {
        // Server has policy with custom level 1 but missing system stats.
        // The fix should merge required fields without dropping level 1.
        let json = r#"{
            "api": {"tag": "api", "services": ["HandlerService", "StatsService"]},
            "stats": {},
            "policy": {
                "levels": {
                    "0": {"statsUserUplink": true, "statsUserDownlink": true},
                    "1": {"handshake": 4, "connIdle": 300}
                }
            },
            "routing": {
                "rules": [{"inboundTag": ["api"], "outboundTag": "api", "type": "field"}]
            },
            "inbounds": [
                {"tag": "api", "port": 8080, "listen": "127.0.0.1", "protocol": "dokodemo-door",
                 "settings": {"address": "127.0.0.1"}},
                {"tag": "vless-in", "protocol": "vless",
                 "settings": {"clients": [{"id": "uuid-1", "flow": "xtls-rprx-vision", "email": "alice@vpn", "level": 0}],
                              "decryption": "none"}}
            ],
            "outbounds": [{"protocol": "freedom", "tag": "direct"}]
        }"#;
        let mut config = ServerConfig::parse(json).unwrap();
        let table = ClientsTable::parse(
            r#"[{"clientId": "uuid-1", "userData": {"clientName": "alice", "creationDate": ""}}]"#,
        )
        .unwrap();

        let modified = enable_api(&mut config, &table).unwrap();
        assert!(modified, "should repair missing system stats");

        let policy = config.raw.get("policy").unwrap();
        // Required fields added
        assert_eq!(policy["system"]["statsInboundUplink"], true);
        assert_eq!(policy["system"]["statsInboundDownlink"], true);
        // Custom level 1 preserved
        assert_eq!(policy["levels"]["1"]["handshake"], 4);
        assert_eq!(policy["levels"]["1"]["connIdle"], 300);
    }

    #[test]
    fn test_enable_api_repairs_broken_routing_rule() {
        // Routing rule has outboundTag "api" but missing inboundTag
        let json = r#"{
            "api": {"tag": "api", "services": ["HandlerService", "StatsService"]},
            "stats": {},
            "policy": {"levels": {"0": {"statsUserUplink": true, "statsUserDownlink": true}},
                       "system": {"statsInboundUplink": true, "statsInboundDownlink": true}},
            "routing": {
                "rules": [{"outboundTag": "api", "type": "field"}]
            },
            "inbounds": [
                {"tag": "api", "port": 8080, "listen": "127.0.0.1", "protocol": "dokodemo-door",
                 "settings": {"address": "127.0.0.1"}},
                {"tag": "vless-in", "protocol": "vless",
                 "settings": {"clients": [{"id": "uuid-1", "flow": "xtls-rprx-vision", "email": "alice@vpn", "level": 0}],
                              "decryption": "none"}}
            ],
            "outbounds": [{"protocol": "freedom", "tag": "direct"}]
        }"#;
        let mut config = ServerConfig::parse(json).unwrap();
        let table = ClientsTable::parse(
            r#"[{"clientId": "uuid-1", "userData": {"clientName": "alice", "creationDate": ""}}]"#,
        )
        .unwrap();

        let modified = enable_api(&mut config, &table).unwrap();
        assert!(modified, "should repair broken routing rule");

        let rules = config.raw["routing"]["rules"].as_array().unwrap();
        assert_eq!(
            rules.len(),
            1,
            "broken rule should be replaced, not duplicated"
        );
        assert_eq!(rules[0]["outboundTag"], "api");
        let inbound_tags = rules[0]["inboundTag"].as_array().unwrap();
        assert!(inbound_tags.iter().any(|t| t == "api"));
    }

    #[test]
    fn test_enable_api_repairs_broken_api_inbound() {
        // API inbound has tag "api" but wrong protocol
        let json = r#"{
            "api": {"tag": "api", "services": ["HandlerService", "StatsService"]},
            "stats": {},
            "policy": {"levels": {"0": {"statsUserUplink": true, "statsUserDownlink": true}},
                       "system": {"statsInboundUplink": true, "statsInboundDownlink": true}},
            "routing": {
                "rules": [{"inboundTag": ["api"], "outboundTag": "api", "type": "field"}]
            },
            "inbounds": [
                {"tag": "api", "port": 9999, "listen": "0.0.0.0", "protocol": "http"},
                {"tag": "vless-in", "protocol": "vless",
                 "settings": {"clients": [{"id": "uuid-1", "flow": "xtls-rprx-vision", "email": "alice@vpn", "level": 0}],
                              "decryption": "none"}}
            ],
            "outbounds": [{"protocol": "freedom", "tag": "direct"}]
        }"#;
        let mut config = ServerConfig::parse(json).unwrap();
        let table = ClientsTable::parse(
            r#"[{"clientId": "uuid-1", "userData": {"clientName": "alice", "creationDate": ""}}]"#,
        )
        .unwrap();

        let modified = enable_api(&mut config, &table).unwrap();
        assert!(modified, "should repair broken api inbound");

        let inbounds = config.raw["inbounds"].as_array().unwrap();
        let api_ib = inbounds.iter().find(|ib| ib["tag"] == "api").unwrap();
        assert_eq!(api_ib["protocol"], "dokodemo-door");
        assert_eq!(api_ib["port"], 8080);
        assert_eq!(api_ib["listen"], "127.0.0.1");
        // Should still have 2 inbounds (replaced, not added)
        assert_eq!(inbounds.len(), 2);
    }

    #[test]
    fn test_enable_api_repairs_wrong_api_tag() {
        // API section has correct services but wrong tag — should be replaced
        let json = r#"{
            "api": {"tag": "wrong", "services": ["HandlerService", "StatsService"]},
            "stats": {},
            "policy": {"levels": {"0": {"statsUserUplink": true, "statsUserDownlink": true}},
                       "system": {"statsInboundUplink": true, "statsInboundDownlink": true}},
            "routing": {
                "rules": [{"inboundTag": ["api"], "outboundTag": "api", "type": "field"}]
            },
            "inbounds": [
                {"tag": "api", "port": 8080, "listen": "127.0.0.1", "protocol": "dokodemo-door",
                 "settings": {"address": "127.0.0.1"}},
                {"tag": "vless-in", "protocol": "vless",
                 "settings": {"clients": [{"id": "uuid-1", "flow": "xtls-rprx-vision", "email": "alice@vpn", "level": 0}],
                              "decryption": "none"}}
            ],
            "outbounds": [{"protocol": "freedom", "tag": "direct"}]
        }"#;
        let mut config = ServerConfig::parse(json).unwrap();
        let table = ClientsTable::parse(
            r#"[{"clientId": "uuid-1", "userData": {"clientName": "alice", "creationDate": ""}}]"#,
        )
        .unwrap();

        let modified = enable_api(&mut config, &table).unwrap();
        assert!(modified, "should repair wrong api tag");
        assert_eq!(config.raw["api"]["tag"], "api");
    }

    #[test]
    fn test_enable_api_handles_malformed_policy_values() {
        // policy, levels, and system are non-object types that should be replaced
        let json = r#"{
            "api": {"tag": "api", "services": ["HandlerService", "StatsService"]},
            "stats": {},
            "policy": null,
            "routing": {
                "rules": [{"inboundTag": ["api"], "outboundTag": "api", "type": "field"}]
            },
            "inbounds": [
                {"tag": "api", "port": 8080, "listen": "127.0.0.1", "protocol": "dokodemo-door",
                 "settings": {"address": "127.0.0.1"}},
                {"tag": "vless-in", "protocol": "vless",
                 "settings": {"clients": [{"id": "uuid-1", "flow": "xtls-rprx-vision", "email": "alice@vpn", "level": 0}],
                              "decryption": "none"}}
            ],
            "outbounds": [{"protocol": "freedom", "tag": "direct"}]
        }"#;
        let mut config = ServerConfig::parse(json).unwrap();
        let table = ClientsTable::parse(
            r#"[{"clientId": "uuid-1", "userData": {"clientName": "alice", "creationDate": ""}}]"#,
        )
        .unwrap();

        // Should not panic on null policy
        let modified = enable_api(&mut config, &table).unwrap();
        assert!(modified);
        assert_eq!(config.raw["policy"]["levels"]["0"]["statsUserUplink"], true);
        assert_eq!(config.raw["policy"]["system"]["statsInboundUplink"], true);
    }

    #[test]
    fn test_enable_api_handles_policy_with_array_levels() {
        // levels is an array instead of an object — should be replaced
        let json = r#"{
            "api": {"tag": "api", "services": ["HandlerService", "StatsService"]},
            "stats": {},
            "policy": {"levels": [], "system": true},
            "routing": {
                "rules": [{"inboundTag": ["api"], "outboundTag": "api", "type": "field"}]
            },
            "inbounds": [
                {"tag": "api", "port": 8080, "listen": "127.0.0.1", "protocol": "dokodemo-door",
                 "settings": {"address": "127.0.0.1"}},
                {"tag": "vless-in", "protocol": "vless",
                 "settings": {"clients": [{"id": "uuid-1", "flow": "xtls-rprx-vision", "email": "alice@vpn", "level": 0}],
                              "decryption": "none"}}
            ],
            "outbounds": [{"protocol": "freedom", "tag": "direct"}]
        }"#;
        let mut config = ServerConfig::parse(json).unwrap();
        let table = ClientsTable::parse(
            r#"[{"clientId": "uuid-1", "userData": {"clientName": "alice", "creationDate": ""}}]"#,
        )
        .unwrap();

        // Should not panic on array levels or boolean system
        let modified = enable_api(&mut config, &table).unwrap();
        assert!(modified);
        assert_eq!(config.raw["policy"]["levels"]["0"]["statsUserUplink"], true);
        assert_eq!(config.raw["policy"]["system"]["statsInboundUplink"], true);
    }

    #[test]
    fn test_enable_api_repairs_malformed_stats_section() {
        // "stats": null should be treated as missing and repaired to {}
        let json = r#"{
            "api": {"tag": "api", "services": ["HandlerService", "StatsService"]},
            "stats": null,
            "policy": {"levels": {"0": {"statsUserUplink": true, "statsUserDownlink": true}},
                        "system": {"statsInboundUplink": true, "statsInboundDownlink": true}},
            "routing": {
                "rules": [{"inboundTag": ["api"], "outboundTag": "api", "type": "field"}]
            },
            "inbounds": [
                {"tag": "api", "port": 8080, "listen": "127.0.0.1", "protocol": "dokodemo-door",
                 "settings": {"address": "127.0.0.1"}},
                {"tag": "vless-in", "protocol": "vless",
                 "settings": {"clients": [{"id": "uuid-1", "flow": "xtls-rprx-vision", "email": "alice@vpn", "level": 0}],
                              "decryption": "none"}}
            ],
            "outbounds": [{"protocol": "freedom", "tag": "direct"}]
        }"#;
        let mut config = ServerConfig::parse(json).unwrap();
        let table = ClientsTable::parse(
            r#"[{"clientId": "uuid-1", "userData": {"clientName": "alice", "creationDate": ""}}]"#,
        )
        .unwrap();

        let modified = enable_api(&mut config, &table).unwrap();
        assert!(modified, "null stats should be repaired");
        assert!(config.raw["stats"].is_object());
    }

    #[test]
    fn test_normalize_vless_tag_updates_routing_rules() {
        // VLESS inbound has a custom tag; a routing rule references it via inboundTag.
        // After normalization, the routing rule should reference "vless-in".
        let json = r#"{
            "routing": {
                "rules": [
                    {"inboundTag": ["api"], "outboundTag": "api", "type": "field"},
                    {"inboundTag": ["my-vless"], "outboundTag": "direct", "type": "field"}
                ]
            },
            "inbounds": [{
                "protocol": "vless",
                "tag": "my-vless",
                "settings": {
                    "clients": [{"id": "uuid-1", "flow": "xtls-rprx-vision"}],
                    "decryption": "none"
                }
            }]
        }"#;
        let mut config = ServerConfig::parse(json).unwrap();

        let changed = normalize_vless_tag(&mut config).unwrap();
        assert!(changed);

        // Routing rule should now reference "vless-in" instead of "my-vless"
        let rules = config.raw["routing"]["rules"].as_array().unwrap();
        let vless_rule = &rules[1];
        let tags = vless_rule["inboundTag"].as_array().unwrap();
        assert_eq!(tags[0], "vless-in");

        // API routing rule should be untouched
        let api_rule = &rules[0];
        let api_tags = api_rule["inboundTag"].as_array().unwrap();
        assert_eq!(api_tags[0], "api");
    }

    #[test]
    fn test_normalize_vless_tag_no_change_when_already_correct() {
        let json = r#"{
            "inbounds": [{
                "protocol": "vless",
                "tag": "vless-in",
                "settings": {
                    "clients": [{"id": "uuid-1", "flow": "xtls-rprx-vision"}],
                    "decryption": "none"
                }
            }]
        }"#;
        let mut config = ServerConfig::parse(json).unwrap();

        let changed = normalize_vless_tag(&mut config).unwrap();
        assert!(!changed);
    }

    #[test]
    fn test_normalize_vless_tag_no_routing_section() {
        // VLESS has wrong tag but no routing section at all — should still normalize
        let json = r#"{
            "inbounds": [{
                "protocol": "vless",
                "tag": "old-tag",
                "settings": {
                    "clients": [{"id": "uuid-1", "flow": "xtls-rprx-vision"}],
                    "decryption": "none"
                }
            }]
        }"#;
        let mut config = ServerConfig::parse(json).unwrap();

        let changed = normalize_vless_tag(&mut config).unwrap();
        assert!(changed);

        let vless = &config.raw["inbounds"].as_array().unwrap()[0];
        assert_eq!(vless["tag"], "vless-in");
    }
}
