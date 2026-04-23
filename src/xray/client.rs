//! `XrayClient`: user mgmt for a native-systemd xray on the bridge/egress host.
//!
//! Edits `/usr/local/etc/xray/config.json` directly via `sudo cat` + `jq`.
//! Does NOT reload xray on mutation — callers must invoke `reload_xray()`
//! after any response has been sent (see the chicken-and-egg note in the
//! Telegram bot).

use crate::backend_trait::XrayBackend;
use crate::error::{AppError, Result};
use crate::native::config_render::{parse_bridge_config, ClientEntry};

pub const NATIVE_CONFIG_PATH: &str = "/usr/local/etc/xray/config.json";
pub const NATIVE_REALITY_PUBKEY_PATH: &str = "/usr/local/etc/xray/reality-public-key";

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
}
