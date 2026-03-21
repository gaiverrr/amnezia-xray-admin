use crate::error::{AppError, Result};
use crate::ssh::SshSession;

use super::config::{read_clients_table, read_server_config, CLIENTS_TABLE_PATH, SERVER_CONFIG_PATH};
use super::types::{
    ClientsTable, ServerConfig, ServerJsonClient, TrafficStats, VlessUrlParams, XrayUser,
};

use base64::Engine;
use uuid::Uuid;

const API_ADDR: &str = "127.0.0.1:8080";
const VLESS_INBOUND_TAG: &str = "vless-in";

/// Server information from Xray.
#[derive(Debug, Clone, Default)]
pub struct ServerInfo {
    pub version: String,
    pub uplink: u64,
    pub downlink: u64,
}

/// Xray API client that communicates via SSH-tunneled docker exec commands.
pub struct XrayApiClient<'a> {
    session: &'a SshSession,
}

impl<'a> XrayApiClient<'a> {
    pub fn new(session: &'a SshSession) -> Self {
        Self { session }
    }

    /// List all users, merged from server.json and clientsTable.
    pub async fn list_users(&self) -> Result<Vec<XrayUser>> {
        let config = read_server_config(self.session).await?;
        let table = read_clients_table(self.session).await?;
        Ok(super::types::merge_users(&config, &table))
    }

    /// Add a new user with the given name. Returns the generated UUID.
    pub async fn add_user(&self, name: &str) -> Result<String> {
        let uuid = Uuid::new_v4().to_string();
        let email = XrayUser::email_from_name(name);

        // 1. Call xray api adu to add user to running instance
        let user_json = build_adu_json(&uuid, &email, VLESS_INBOUND_TAG);
        self.exec_api_adu(&user_json).await?;

        // 2. Update server.json on disk
        let mut config = read_server_config(self.session).await?;
        let client = ServerJsonClient {
            id: uuid.clone(),
            flow: "xtls-rprx-vision".to_string(),
            email: Some(email),
            level: Some(0),
        };
        config.add_client(&client)?;
        self.write_server_config(&config).await?;

        // 3. Update clientsTable
        let mut table = read_clients_table(self.session).await?;
        table.add(uuid.clone(), name.to_string());
        self.write_clients_table(&table).await?;

        Ok(uuid)
    }

    /// Remove a user by UUID.
    pub async fn remove_user(&self, uuid: &str) -> Result<()> {
        // Find the user's email first
        let config = read_server_config(self.session).await?;
        let table = read_clients_table(self.session).await?;

        let email = config
            .clients()
            .iter()
            .find(|c| c.id == uuid)
            .and_then(|c| c.email.clone())
            .or_else(|| table.name_for_uuid(uuid).map(|n| XrayUser::email_from_name(n)))
            .ok_or_else(|| AppError::Xray(format!("user {} not found", uuid)))?;

        // 1. Call xray api rmu to remove from running instance
        self.exec_api_rmu(&email).await?;

        // 2. Update server.json
        let mut config = config;
        config.remove_client(uuid)?;
        self.write_server_config(&config).await?;

        // 3. Update clientsTable
        let mut table = table;
        table.remove(uuid);
        self.write_clients_table(&table).await?;

        Ok(())
    }

    /// Get traffic stats for a user by email.
    pub async fn get_user_stats(&self, email: &str) -> Result<TrafficStats> {
        let uplink_cmd = build_stats_cmd(email, "uplink");
        let downlink_cmd = build_stats_cmd(email, "downlink");

        let up_result = self.session.exec_in_container(&uplink_cmd).await?;
        let down_result = self.session.exec_in_container(&downlink_cmd).await?;

        let uplink = parse_stat_value(&up_result.stdout).unwrap_or(0);
        let downlink = parse_stat_value(&down_result.stdout).unwrap_or(0);

        Ok(TrafficStats { uplink, downlink })
    }

    /// Get online connection count for a user.
    pub async fn get_online_count(&self, email: &str) -> Result<u32> {
        let cmd = build_online_cmd(email);
        let result = self.session.exec_in_container(&cmd).await?;
        Ok(parse_stat_value(&result.stdout).unwrap_or(0) as u32)
    }

    /// Get list of online IPs for a user.
    pub async fn get_online_ips(&self, email: &str) -> Result<Vec<String>> {
        let cmd = build_online_ip_list_cmd(email);
        let result = self.session.exec_in_container(&cmd).await?;
        Ok(parse_ip_list(&result.stdout))
    }

    /// Get server info (version, total traffic).
    pub async fn get_server_info(&self) -> Result<ServerInfo> {
        let version_result = self.session.exec_in_container("xray version").await?;
        let version = parse_version(&version_result.stdout);

        let up_cmd = build_inbound_stats_cmd(VLESS_INBOUND_TAG, "uplink");
        let down_cmd = build_inbound_stats_cmd(VLESS_INBOUND_TAG, "downlink");

        let up_result = self.session.exec_in_container(&up_cmd).await?;
        let down_result = self.session.exec_in_container(&down_cmd).await?;

        Ok(ServerInfo {
            version,
            uplink: parse_stat_value(&up_result.stdout).unwrap_or(0),
            downlink: parse_stat_value(&down_result.stdout).unwrap_or(0),
        })
    }

    // -- internal helpers --

    async fn exec_api_adu(&self, user_json: &str) -> Result<()> {
        // Use base64 to safely pass JSON through shell layers
        let b64 = base64::engine::general_purpose::STANDARD.encode(user_json.as_bytes());
        let cmd = format!(
            "sh -c 'echo {} | base64 -d > /tmp/_adu.json && xray api adu -s {} /tmp/_adu.json; rc=$?; rm -f /tmp/_adu.json; exit $rc'",
            b64, API_ADDR
        );
        let result = self.session.exec_in_container(&cmd).await?;
        if !result.success() {
            return Err(AppError::Xray(format!(
                "adu failed: {}",
                result.stderr.trim()
            )));
        }
        Ok(())
    }

    async fn exec_api_rmu(&self, email: &str) -> Result<()> {
        let cmd = build_rmu_cmd(email);
        let result = self.session.exec_in_container(&cmd).await?;
        if !result.success() {
            return Err(AppError::Xray(format!(
                "rmu failed: {}",
                result.stderr.trim()
            )));
        }
        Ok(())
    }

    async fn write_server_config(&self, config: &ServerConfig) -> Result<()> {
        let json = config.to_json();
        let escaped = json.replace('\'', "'\\''");
        let cmd = format!("printf '%s' '{}' > {}", escaped, SERVER_CONFIG_PATH);
        let result = self.session.exec_command(&cmd).await?;
        if !result.success() {
            return Err(AppError::Xray(format!(
                "failed to write server config: {}",
                result.stderr
            )));
        }
        Ok(())
    }

    async fn write_clients_table(&self, table: &ClientsTable) -> Result<()> {
        let json = table.to_json();
        let escaped = json.replace('\'', "'\\''");
        let cmd = format!("printf '%s' '{}' > {}", escaped, CLIENTS_TABLE_PATH);
        let result = self.session.exec_command(&cmd).await?;
        if !result.success() {
            return Err(AppError::Xray(format!(
                "failed to write clients table: {}",
                result.stderr
            )));
        }
        Ok(())
    }
}

// -- Command construction (pure functions, testable) --

/// Build the JSON payload for `xray api adu`.
pub fn build_adu_json(uuid: &str, email: &str, inbound_tag: &str) -> String {
    serde_json::json!({
        "inboundTag": inbound_tag,
        "user": {
            "email": email,
            "level": 0,
            "account": {
                "id": uuid,
                "flow": "xtls-rprx-vision"
            }
        }
    })
    .to_string()
}

/// Build the `xray api rmu` command string.
pub fn build_rmu_cmd(email: &str) -> String {
    format!("xray api rmu -s {} -email {}", API_ADDR, email)
}

/// Build the `xray api stats` command for user traffic.
pub fn build_stats_cmd(email: &str, direction: &str) -> String {
    format!(
        "xray api stats -s {} -name 'user>>>{}>>>traffic>>>{}'",
        API_ADDR, email, direction
    )
}

/// Build the `xray api stats` command for inbound traffic.
pub fn build_inbound_stats_cmd(inbound_tag: &str, direction: &str) -> String {
    format!(
        "xray api stats -s {} -name 'inbound>>>{}>>>traffic>>>{}'",
        API_ADDR, inbound_tag, direction
    )
}

/// Build the `xray api statsonline` command.
pub fn build_online_cmd(email: &str) -> String {
    format!("xray api statsonline -s {} -email {}", API_ADDR, email)
}

/// Build the `xray api statsonlineiplist` command.
pub fn build_online_ip_list_cmd(email: &str) -> String {
    format!(
        "xray api statsonlineiplist -s {} -email {}",
        API_ADDR, email
    )
}

// -- vless:// URL generation --

/// Generate a vless:// URL for client import.
///
/// Format: `vless://<uuid>@<host>:<port>?encryption=none&flow=xtls-rprx-vision&type=tcp&security=reality&sni=<sni>&fp=chrome&pbk=<pubkey>&sid=<shortid>#<name>`
pub fn generate_vless_url(params: &VlessUrlParams) -> String {
    let fragment = urlencod_fragment(&params.name);
    format!(
        "vless://{}@{}:{}?encryption=none&flow=xtls-rprx-vision&type=tcp&security=reality&sni={}&fp=chrome&pbk={}&sid={}#{}",
        params.uuid, params.host, params.port, params.sni, params.public_key, params.short_id, fragment
    )
}

/// Percent-encode a fragment string for use in a URL.
/// Only encodes characters that are not allowed in URL fragments.
fn urlencod_fragment(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' | '!' | '\'' | '(' | ')' | '*' => {
                result.push(ch);
            }
            ' ' => result.push_str("%20"),
            _ => {
                for byte in ch.to_string().as_bytes() {
                    result.push_str(&format!("%{:02X}", byte));
                }
            }
        }
    }
    result
}

// -- Response parsing (pure functions, testable) --

/// Parse a stat value from xray API output.
///
/// Expected format:
/// ```text
/// stat: {
///   name: "..."
///   value: 12345
/// }
/// ```
pub fn parse_stat_value(output: &str) -> Option<u64> {
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("value:") {
            let val_str = rest.trim();
            if let Ok(val) = val_str.parse::<i64>() {
                // Stats can be negative after reset; treat as 0
                return Some(val.max(0) as u64);
            }
        }
    }
    None
}

/// Parse the Xray version string from `xray version` output.
///
/// Expected format: "Xray 25.8.3 (Xray, Pair of Penetrating Rays) ..."
pub fn parse_version(output: &str) -> String {
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Xray ") {
            // Extract "25.8.3" from "Xray 25.8.3 (...)"
            if let Some(rest) = trimmed.strip_prefix("Xray ") {
                if let Some(version) = rest.split_whitespace().next() {
                    return version.to_string();
                }
            }
        }
    }
    "unknown".to_string()
}

/// Parse online IP list from `xray api statsonlineiplist` output.
///
/// Expected format: multiple stat entries where the IP is embedded in the name:
/// ```text
/// stat: {
///   name: "user>>>email@vpn>>>online>>>ip>>>1.2.3.4"
///   value: 1234567890
/// }
/// ```
pub fn parse_ip_list(output: &str) -> Vec<String> {
    let mut ips = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("name:") {
            let name = rest.trim().trim_matches('"');
            // Look for >>>ip>>> segment
            if let Some(ip_part) = name.split(">>>ip>>>").nth(1) {
                let ip = ip_part.trim_matches('"');
                if !ip.is_empty() {
                    ips.push(ip.to_string());
                }
            }
        }
    }
    ips
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Command construction tests --

    #[test]
    fn test_build_adu_json() {
        let json = build_adu_json("test-uuid", "alice@vpn", "vless-in");
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["inboundTag"], "vless-in");
        assert_eq!(parsed["user"]["email"], "alice@vpn");
        assert_eq!(parsed["user"]["level"], 0);
        assert_eq!(parsed["user"]["account"]["id"], "test-uuid");
        assert_eq!(parsed["user"]["account"]["flow"], "xtls-rprx-vision");
    }

    #[test]
    fn test_build_adu_json_special_chars_in_name() {
        let json = build_adu_json("uuid-123", "bob's-phone@vpn", "vless-in");
        // Should still be valid JSON
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["user"]["email"], "bob's-phone@vpn");
    }

    #[test]
    fn test_build_rmu_cmd() {
        let cmd = build_rmu_cmd("alice@vpn");
        assert_eq!(
            cmd,
            "xray api rmu -s 127.0.0.1:8080 -email alice@vpn"
        );
    }

    #[test]
    fn test_build_stats_cmd_uplink() {
        let cmd = build_stats_cmd("alice@vpn", "uplink");
        assert_eq!(
            cmd,
            "xray api stats -s 127.0.0.1:8080 -name 'user>>>alice@vpn>>>traffic>>>uplink'"
        );
    }

    #[test]
    fn test_build_stats_cmd_downlink() {
        let cmd = build_stats_cmd("alice@vpn", "downlink");
        assert!(cmd.contains("user>>>alice@vpn>>>traffic>>>downlink"));
    }

    #[test]
    fn test_build_inbound_stats_cmd() {
        let cmd = build_inbound_stats_cmd("vless-in", "uplink");
        assert_eq!(
            cmd,
            "xray api stats -s 127.0.0.1:8080 -name 'inbound>>>vless-in>>>traffic>>>uplink'"
        );
    }

    #[test]
    fn test_build_online_cmd() {
        let cmd = build_online_cmd("bob@vpn");
        assert_eq!(
            cmd,
            "xray api statsonline -s 127.0.0.1:8080 -email bob@vpn"
        );
    }

    #[test]
    fn test_build_online_ip_list_cmd() {
        let cmd = build_online_ip_list_cmd("bob@vpn");
        assert_eq!(
            cmd,
            "xray api statsonlineiplist -s 127.0.0.1:8080 -email bob@vpn"
        );
    }

    // -- Response parsing tests --

    #[test]
    fn test_parse_stat_value_basic() {
        let output = r#"stat: {
  name: "user>>>alice@vpn>>>traffic>>>downlink"
  value: 123456
}"#;
        assert_eq!(parse_stat_value(output), Some(123456));
    }

    #[test]
    fn test_parse_stat_value_large_number() {
        let output = "stat: {\n  name: \"test\"\n  value: 9876543210\n}";
        assert_eq!(parse_stat_value(output), Some(9876543210));
    }

    #[test]
    fn test_parse_stat_value_zero() {
        let output = "stat: {\n  name: \"test\"\n  value: 0\n}";
        assert_eq!(parse_stat_value(output), Some(0));
    }

    #[test]
    fn test_parse_stat_value_negative_becomes_zero() {
        let output = "stat: {\n  name: \"test\"\n  value: -100\n}";
        assert_eq!(parse_stat_value(output), Some(0));
    }

    #[test]
    fn test_parse_stat_value_empty_output() {
        assert_eq!(parse_stat_value(""), None);
    }

    #[test]
    fn test_parse_stat_value_no_value_line() {
        let output = "stat: {\n  name: \"test\"\n}";
        assert_eq!(parse_stat_value(output), None);
    }

    #[test]
    fn test_parse_stat_value_invalid_number() {
        let output = "stat: {\n  name: \"test\"\n  value: notanumber\n}";
        assert_eq!(parse_stat_value(output), None);
    }

    #[test]
    fn test_parse_version_standard() {
        let output =
            "Xray 25.8.3 (Xray, Pair of Penetrating Rays) Custom (go1.22.5 linux/amd64)\nA unified platform for anti-censorship.";
        assert_eq!(parse_version(output), "25.8.3");
    }

    #[test]
    fn test_parse_version_simple() {
        let output = "Xray 1.2.3\n";
        assert_eq!(parse_version(output), "1.2.3");
    }

    #[test]
    fn test_parse_version_empty() {
        assert_eq!(parse_version(""), "unknown");
    }

    #[test]
    fn test_parse_version_no_xray_prefix() {
        assert_eq!(parse_version("some other output"), "unknown");
    }

    #[test]
    fn test_parse_ip_list_single() {
        let output = r#"stat: {
  name: "user>>>alice@vpn>>>online>>>ip>>>1.2.3.4"
  value: 1700000000
}"#;
        let ips = parse_ip_list(output);
        assert_eq!(ips, vec!["1.2.3.4"]);
    }

    #[test]
    fn test_parse_ip_list_multiple() {
        let output = r#"stat: {
  name: "user>>>alice@vpn>>>online>>>ip>>>1.2.3.4"
  value: 1700000000
}
stat: {
  name: "user>>>alice@vpn>>>online>>>ip>>>5.6.7.8"
  value: 1700000001
}"#;
        let ips = parse_ip_list(output);
        assert_eq!(ips, vec!["1.2.3.4", "5.6.7.8"]);
    }

    #[test]
    fn test_parse_ip_list_empty() {
        assert_eq!(parse_ip_list(""), Vec::<String>::new());
    }

    #[test]
    fn test_parse_ip_list_no_ip_entries() {
        let output = r#"stat: {
  name: "user>>>alice@vpn>>>traffic>>>downlink"
  value: 12345
}"#;
        assert_eq!(parse_ip_list(output), Vec::<String>::new());
    }

    #[test]
    fn test_parse_ip_list_ipv6() {
        let output = r#"stat: {
  name: "user>>>alice@vpn>>>online>>>ip>>>2001:db8::1"
  value: 1700000000
}"#;
        let ips = parse_ip_list(output);
        assert_eq!(ips, vec!["2001:db8::1"]);
    }

    // -- Integration-like tests for command/response roundtrip --

    #[test]
    fn test_adu_json_is_valid_for_api() {
        let json = build_adu_json(
            "550e8400-e29b-41d4-a716-446655440000",
            "testuser@vpn",
            "vless-in",
        );
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Verify structure matches what xray api expects
        assert!(parsed.get("inboundTag").is_some());
        assert!(parsed.get("user").is_some());
        assert!(parsed["user"].get("email").is_some());
        assert!(parsed["user"].get("account").is_some());
        assert!(parsed["user"]["account"].get("id").is_some());
        assert!(parsed["user"]["account"].get("flow").is_some());
    }

    #[test]
    fn test_stats_cmd_uses_correct_separator() {
        // Xray uses >>> as separator in stat names
        let cmd = build_stats_cmd("test@vpn", "uplink");
        assert!(cmd.contains(">>>"));
        // Should have exactly 3 separators: user>>>email>>>traffic>>>direction
        let stat_name = cmd.split('\'').nth(1).unwrap();
        assert_eq!(stat_name.matches(">>>").count(), 3);
    }

    #[test]
    fn test_inbound_stats_cmd_uses_correct_separator() {
        let cmd = build_inbound_stats_cmd("vless-in", "downlink");
        let stat_name = cmd.split('\'').nth(1).unwrap();
        assert_eq!(stat_name.matches(">>>").count(), 3);
    }

    #[test]
    fn test_server_info_default() {
        let info = ServerInfo::default();
        assert_eq!(info.version, "");
        assert_eq!(info.uplink, 0);
        assert_eq!(info.downlink, 0);
    }

    #[test]
    fn test_parse_stat_value_with_whitespace() {
        let output = "  value:   42  \n";
        assert_eq!(parse_stat_value(output), Some(42));
    }

    #[test]
    fn test_parse_version_with_leading_whitespace() {
        let output = "  Xray 25.8.3 (something)\n";
        // Our parser trims lines
        assert_eq!(parse_version(output), "25.8.3");
    }

    // -- vless:// URL generation tests --

    #[test]
    fn test_generate_vless_url_basic() {
        let params = VlessUrlParams {
            uuid: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            host: "1.2.3.4".to_string(),
            port: 443,
            sni: "www.googletagmanager.com".to_string(),
            public_key: "testpublickey123".to_string(),
            short_id: "abcd1234".to_string(),
            name: "TestUser".to_string(),
        };
        let url = generate_vless_url(&params);

        assert!(url.starts_with("vless://550e8400-e29b-41d4-a716-446655440000@1.2.3.4:443?"));
        assert!(url.contains("encryption=none"));
        assert!(url.contains("flow=xtls-rprx-vision"));
        assert!(url.contains("type=tcp"));
        assert!(url.contains("security=reality"));
        assert!(url.contains("sni=www.googletagmanager.com"));
        assert!(url.contains("fp=chrome"));
        assert!(url.contains("pbk=testpublickey123"));
        assert!(url.contains("sid=abcd1234"));
        assert!(url.ends_with("#TestUser"));
    }

    #[test]
    fn test_generate_vless_url_exact_format() {
        let params = VlessUrlParams {
            uuid: "uuid-123".to_string(),
            host: "10.0.0.1".to_string(),
            port: 8443,
            sni: "example.com".to_string(),
            public_key: "pk123".to_string(),
            short_id: "sid1".to_string(),
            name: "alice".to_string(),
        };
        let expected = "vless://uuid-123@10.0.0.1:8443?encryption=none&flow=xtls-rprx-vision&type=tcp&security=reality&sni=example.com&fp=chrome&pbk=pk123&sid=sid1#alice";
        assert_eq!(generate_vless_url(&params), expected);
    }

    #[test]
    fn test_generate_vless_url_name_with_spaces() {
        let params = VlessUrlParams {
            uuid: "uuid-123".to_string(),
            host: "1.2.3.4".to_string(),
            port: 443,
            sni: "example.com".to_string(),
            public_key: "pk".to_string(),
            short_id: "sid".to_string(),
            name: "My Phone".to_string(),
        };
        let url = generate_vless_url(&params);
        assert!(url.ends_with("#My%20Phone"));
    }

    #[test]
    fn test_generate_vless_url_name_with_special_chars() {
        let params = VlessUrlParams {
            uuid: "uuid-123".to_string(),
            host: "1.2.3.4".to_string(),
            port: 443,
            sni: "example.com".to_string(),
            public_key: "pk".to_string(),
            short_id: "sid".to_string(),
            name: "bob's-phone".to_string(),
        };
        let url = generate_vless_url(&params);
        // Apostrophe is allowed in fragments
        assert!(url.ends_with("#bob's-phone"));
    }

    #[test]
    fn test_generate_vless_url_ipv6_host() {
        let params = VlessUrlParams {
            uuid: "uuid-123".to_string(),
            host: "2001:db8::1".to_string(),
            port: 443,
            sni: "example.com".to_string(),
            public_key: "pk".to_string(),
            short_id: "sid".to_string(),
            name: "test".to_string(),
        };
        let url = generate_vless_url(&params);
        assert!(url.contains("@2001:db8::1:443"));
    }

    #[test]
    fn test_urlencod_fragment_plain() {
        assert_eq!(urlencod_fragment("alice"), "alice");
    }

    #[test]
    fn test_urlencod_fragment_spaces() {
        assert_eq!(urlencod_fragment("my phone"), "my%20phone");
    }

    #[test]
    fn test_urlencod_fragment_unicode() {
        let encoded = urlencod_fragment("тест");
        assert!(encoded.contains('%'));
        assert!(!encoded.contains("тест"));
    }

    #[test]
    fn test_urlencod_fragment_allowed_chars() {
        assert_eq!(urlencod_fragment("a-b_c.d~e"), "a-b_c.d~e");
    }
}
