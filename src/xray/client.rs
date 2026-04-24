//! `XrayClient`: user mgmt for a native-systemd xray on the bridge/egress host.
//!
//! Edits `/usr/local/etc/xray/config.json` directly via `sudo cat` + `jq`.
//! Does NOT reload xray on mutation — callers must invoke `reload_xray()`
//! after any response has been sent (see the chicken-and-egg note in the
//! Telegram bot).

use std::collections::HashMap;

use crate::backend_trait::XrayBackend;
use crate::error::{AppError, Result};
use crate::xray::config_render::{parse_bridge_config, ClientEntry};
use crate::xray::types::TrafficStats;

pub const NATIVE_CONFIG_PATH: &str = "/usr/local/etc/xray/config.json";
pub const NATIVE_REALITY_PUBKEY_PATH: &str = "/usr/local/etc/xray/reality-public-key";
/// Xray's gRPC API listens here on the bridge (see `api` + `dokodemo-door` inbound).
pub const NATIVE_API_ADDRESS: &str = "127.0.0.1:8080";

/// Public Reality params that bot needs to render vless URLs for users.
#[derive(Debug, Clone)]
pub struct BridgePublicParams {
    pub port: u16,
    pub sni: String,
    pub short_id: String,
    pub path: String,
    pub public_key: String,
}

pub struct XrayClient<'a> {
    backend: &'a dyn XrayBackend,
}

impl<'a> XrayClient<'a> {
    pub fn new(backend: &'a dyn XrayBackend) -> Self {
        Self { backend }
    }

    /// Read the current bridge config and return the live client list.
    pub async fn list_clients(&self) -> Result<Vec<ClientEntry>> {
        let out = self
            .backend
            .exec_on_host(&format!("sudo cat {NATIVE_CONFIG_PATH}"))
            .await?;
        if !out.success() {
            return Err(AppError::Config(format!(
                "read bridge config: {}",
                out.stderr
            )));
        }
        let parsed = parse_bridge_config(&out.stdout)?;
        Ok(parsed.clients)
    }

    /// Add a user (generates a fresh UUID). Name becomes email `<name>@vpn`.
    /// Backs up config, patches via jq. Does NOT reload xray — caller must
    /// invoke `reload_xray` after any response has been sent, because the
    /// bot's own HTTP proxy runs through xray and restart momentarily kills
    /// it, racing with the response send.
    pub async fn add_client(&self, name: &str) -> Result<ClientEntry> {
        validate_name(name)?;
        let email = format!("{name}@vpn");
        // Reject duplicates early
        let existing = self.list_clients().await?;
        if existing.iter().any(|c| c.email == email) {
            return Err(AppError::Config(format!("user '{name}' already exists")));
        }
        let uuid = uuid::Uuid::new_v4().to_string();
        let cmd = format!(
            "sudo cp {cfg} {cfg}.bak-$(date +%s) && \
             sudo jq --arg uuid '{uuid}' --arg email '{email}' \
               '.inbounds[0].settings.clients += [{{id: $uuid, email: $email}}]' \
               {cfg} | sudo tee {cfg}.new > /dev/null && \
             sudo mv {cfg}.new {cfg}",
            cfg = NATIVE_CONFIG_PATH,
            uuid = uuid,
            email = email,
        );
        let out = self.backend.exec_on_host(&cmd).await?;
        if !out.success() {
            return Err(AppError::Config(format!("add client: {}", out.stderr)));
        }
        Ok(ClientEntry { uuid, email })
    }

    /// Remove by user name (email prefix). Backs up, patches config. Does NOT
    /// reload xray — see `add_client` docs.
    pub async fn remove_client(&self, name: &str) -> Result<()> {
        validate_name(name)?;
        let email = format!("{name}@vpn");
        let cmd = format!(
            "sudo cp {cfg} {cfg}.bak-$(date +%s) && \
             sudo jq --arg email '{email}' \
               '.inbounds[0].settings.clients |= map(select(.email != $email))' \
               {cfg} | sudo tee {cfg}.new > /dev/null && \
             sudo mv {cfg}.new {cfg}",
            cfg = NATIVE_CONFIG_PATH,
            email = email,
        );
        let out = self.backend.exec_on_host(&cmd).await?;
        if !out.success() {
            return Err(AppError::Config(format!("remove client: {}", out.stderr)));
        }
        Ok(())
    }

    /// Reload xray so it picks up a config edit. Call AFTER any Telegram
    /// response has been sent, because the bot proxies its outbound through
    /// xray itself.
    pub async fn reload_xray(&self) -> Result<()> {
        let out = self
            .backend
            .exec_on_host("sudo systemctl reload-or-restart xray")
            .await?;
        if !out.success() {
            return Err(AppError::Config(format!("reload xray: {}", out.stderr)));
        }
        Ok(())
    }

    /// Public params needed to render a vless URL for any client on this bridge.
    pub async fn bridge_public_params(&self) -> Result<BridgePublicParams> {
        let out = self
            .backend
            .exec_on_host(&format!("sudo cat {NATIVE_CONFIG_PATH}"))
            .await?;
        if !out.success() {
            return Err(AppError::Config(format!(
                "read bridge config: {}",
                out.stderr
            )));
        }
        let v: serde_json::Value = serde_json::from_str(&out.stdout)
            .map_err(|e| AppError::Config(format!("parse bridge config: {e}")))?;
        let inbound = &v["inbounds"][0];
        let stream = &inbound["streamSettings"];

        let port = inbound["port"].as_u64().unwrap_or(443) as u16;
        let sni = stream["realitySettings"]["serverNames"][0]
            .as_str()
            .unwrap_or("")
            .to_string();
        let short_id = stream["realitySettings"]["shortIds"][0]
            .as_str()
            .unwrap_or("")
            .to_string();
        let path = stream["xhttpSettings"]["path"]
            .as_str()
            .unwrap_or("")
            .to_string();

        // Reality public key isn't in server.json (only the private is). We
        // store it in a sidecar file during bridge setup.
        let pub_out = self
            .backend
            .exec_on_host(&format!("sudo cat {NATIVE_REALITY_PUBKEY_PATH}"))
            .await?;
        if !pub_out.success() {
            return Err(AppError::Config(format!(
                "read reality public key at {NATIVE_REALITY_PUBKEY_PATH}: {}\n\
                 During bridge setup, run: echo <pubkey> | sudo tee {NATIVE_REALITY_PUBKEY_PATH}",
                pub_out.stderr
            )));
        }
        let public_key = pub_out.stdout.trim().to_string();
        if public_key.is_empty() {
            return Err(AppError::Config(format!(
                "reality public key file {NATIVE_REALITY_PUBKEY_PATH} is empty"
            )));
        }

        Ok(BridgePublicParams {
            port,
            sni,
            short_id,
            path,
            public_key,
        })
    }

    /// Get the UUID for an existing client by name. Returns Err if not found.
    pub async fn get_uuid(&self, name: &str) -> Result<String> {
        let email = format!("{name}@vpn");
        let clients = self.list_clients().await?;
        clients
            .into_iter()
            .find(|c| c.email == email)
            .map(|c| c.uuid)
            .ok_or_else(|| AppError::Config(format!("user '{name}' not found")))
    }

    /// Query xray's `statsquery` API for all per-user traffic counters. Returns
    /// a map keyed by the full email (`name@vpn`).
    ///
    /// On any transport/parse failure this returns an empty map rather than an
    /// error — stats are informational and the bot should keep working even if
    /// the api inbound is temporarily unreachable.
    pub async fn get_all_user_stats(&self) -> HashMap<String, TrafficStats> {
        let cmd = format!("xray api statsquery --server={NATIVE_API_ADDRESS} -pattern 'user>>>'");
        let Ok(out) = self.backend.exec_on_host(&cmd).await else {
            return HashMap::new();
        };
        if !out.success() {
            return HashMap::new();
        }
        parse_user_stats(&out.stdout).unwrap_or_default()
    }

    /// Aggregate uplink/downlink for a given inbound tag (e.g. `client-in`).
    /// Returns `(0, 0)` on any failure — see rationale on `get_all_user_stats`.
    pub async fn get_inbound_stats(&self, tag: &str) -> (u64, u64) {
        let cmd = format!(
            "xray api statsquery --server={NATIVE_API_ADDRESS} -pattern 'inbound>>>{tag}>>>'"
        );
        let Ok(out) = self.backend.exec_on_host(&cmd).await else {
            return (0, 0);
        };
        if !out.success() {
            return (0, 0);
        }
        parse_inbound_stats(&out.stdout).unwrap_or((0, 0))
    }
}

/// Parse the `{"stat": [{"name": "user>>>rita@vpn>>>traffic>>>downlink", "value": N}, ...]}`
/// payload into a map keyed by email. Unknown shapes are skipped silently.
pub(crate) fn parse_user_stats(json: &str) -> Result<HashMap<String, TrafficStats>> {
    let v: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| AppError::Config(format!("parse statsquery: {e}")))?;
    let mut map: HashMap<String, TrafficStats> = HashMap::new();
    let Some(arr) = v.get("stat").and_then(|s| s.as_array()) else {
        return Ok(map);
    };
    for entry in arr {
        let Some(name) = entry.get("name").and_then(|n| n.as_str()) else {
            continue;
        };
        let Some(value) = entry.get("value").and_then(|x| x.as_u64()) else {
            continue;
        };
        // Expected: user>>>{email}>>>traffic>>>{uplink|downlink}
        let parts: Vec<&str> = name.split(">>>").collect();
        if parts.len() != 4 || parts[0] != "user" || parts[2] != "traffic" {
            continue;
        }
        let slot = map.entry(parts[1].to_string()).or_default();
        match parts[3] {
            "uplink" => slot.uplink = value,
            "downlink" => slot.downlink = value,
            _ => {}
        }
    }
    Ok(map)
}

/// Sum uplink/downlink across all matching inbound entries in a statsquery
/// response. Expected shape: `inbound>>>{tag}>>>traffic>>>{uplink|downlink}`.
pub(crate) fn parse_inbound_stats(json: &str) -> Result<(u64, u64)> {
    let v: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| AppError::Config(format!("parse statsquery: {e}")))?;
    let mut uplink = 0u64;
    let mut downlink = 0u64;
    let Some(arr) = v.get("stat").and_then(|s| s.as_array()) else {
        return Ok((0, 0));
    };
    for entry in arr {
        let Some(name) = entry.get("name").and_then(|n| n.as_str()) else {
            continue;
        };
        let Some(value) = entry.get("value").and_then(|x| x.as_u64()) else {
            continue;
        };
        let parts: Vec<&str> = name.split(">>>").collect();
        if parts.len() != 4 {
            continue;
        }
        match parts[3] {
            "uplink" => uplink += value,
            "downlink" => downlink += value,
            _ => {}
        }
    }
    Ok((uplink, downlink))
}

/// Reject names that could break shell quoting or jq arg parsing.
fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(AppError::Config("user name is empty".into()));
    }
    // Allow letters, digits, spaces, dash, underscore, brackets, dot.
    // Reject anything that could terminate our jq arg string.
    let forbidden = ['\'', '"', '\\', '\n', '\r', '\0'];
    if name.chars().any(|c| forbidden.contains(&c)) {
        return Err(AppError::Config(format!(
            "user name contains forbidden character: {name:?}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_name_rejects_quotes() {
        assert!(validate_name("foo'bar").is_err());
        assert!(validate_name("foo\"bar").is_err());
        assert!(validate_name("").is_err());
        assert!(validate_name("ok name").is_ok());
        assert!(validate_name("Admin [macOS Tahoe (26.3.1)]").is_ok());
        assert!(validate_name("user-1_a.b").is_ok());
    }

    #[test]
    fn parse_user_stats_groups_by_email() {
        let json = r#"{
            "stat": [
                { "name": "user>>>rita@vpn>>>traffic>>>downlink", "value": 1000 },
                { "name": "user>>>rita@vpn>>>traffic>>>uplink",   "value": 200  },
                { "name": "user>>>ivan@vpn>>>traffic>>>downlink", "value": 50   }
            ]
        }"#;
        let map = parse_user_stats(json).unwrap();
        assert_eq!(map["rita@vpn"].uplink, 200);
        assert_eq!(map["rita@vpn"].downlink, 1000);
        assert_eq!(map["ivan@vpn"].uplink, 0);
        assert_eq!(map["ivan@vpn"].downlink, 50);
    }

    #[test]
    fn parse_user_stats_empty() {
        assert!(parse_user_stats(r#"{"stat":[]}"#).unwrap().is_empty());
        assert!(parse_user_stats(r#"{}"#).unwrap().is_empty());
    }

    #[test]
    fn parse_user_stats_skips_malformed_entries() {
        // Each of these entries is individually malformed; parser should skip
        // them silently without returning an error.
        let json = r#"{
            "stat": [
                { "name": "user>>>short", "value": 1 },
                { "name": "outbound>>>foo>>>traffic>>>uplink", "value": 2 },
                { "name": "user>>>x@y>>>traffic>>>uplink" },
                { "value": 3 },
                "not-an-object"
            ]
        }"#;
        let map = parse_user_stats(json).unwrap();
        assert!(map.is_empty());
    }

    #[test]
    fn parse_user_stats_rejects_non_json() {
        assert!(parse_user_stats("not json").is_err());
    }

    #[test]
    fn parse_inbound_stats_sums_uplink_and_downlink() {
        let json = r#"{
            "stat": [
                { "name": "inbound>>>client-in>>>traffic>>>uplink",   "value": 10 },
                { "name": "inbound>>>client-in>>>traffic>>>downlink", "value": 90 }
            ]
        }"#;
        let (up, down) = parse_inbound_stats(json).unwrap();
        assert_eq!(up, 10);
        assert_eq!(down, 90);
    }

    #[test]
    fn parse_inbound_stats_empty() {
        assert_eq!(parse_inbound_stats(r#"{"stat":[]}"#).unwrap(), (0, 0));
    }
}
