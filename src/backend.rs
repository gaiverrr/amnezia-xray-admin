//! Async backend operations for the TUI.
//!
//! Provides the bridge between the synchronous event loop and async SSH/Xray operations.
//! Operations are spawned on a tokio runtime and results are sent back via channels.

use std::sync::mpsc;

use crate::config::Config;
use crate::error::AppError;
use crate::ssh::{expand_tilde, resolve_ssh_host, SshSession};
use crate::xray::client::{generate_vless_url, ServerInfo, XrayApiClient};
use crate::xray::config::{ensure_api_enabled, read_server_config};
use crate::xray::types::{VlessUrlParams, XrayUser};

/// Path to the Xray public key file on the server (used for vless:// URLs).
const PUBLIC_KEY_PATH: &str = "/opt/amnezia/xray/xray_public.key";

/// Why a vless URL was requested
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VlessUrlIntent {
    /// User pressed [c] to copy URL to clipboard
    Clipboard,
    /// User pressed [q] to show QR code
    Qr,
}

/// Messages sent from backend async tasks to the UI thread.
pub enum BackendMsg {
    /// Dashboard data loaded (users + server info)
    DashboardData(Result<DashboardData, String>),
    /// SSH connection test result (Ok = xray version string)
    ConnectionTest(Result<String, String>),
    /// User added successfully
    UserAdded(Result<AddedUser, String>),
    /// User deleted successfully (Ok = deleted uuid)
    UserDeleted(Result<String, String>),
    /// Vless URL generated for an existing user (copy or QR)
    VlessUrl(Result<AddedUser, String>, VlessUrlIntent),
    /// Online IPs fetched for a user (Ok = (uuid, ips), Err = (uuid, error))
    OnlineIps(Result<(String, Vec<String>), (String, String)>),
}

/// Dashboard data bundle
pub struct DashboardData {
    pub users: Vec<XrayUser>,
    pub server_info: ServerInfo,
}

/// Result of adding a user
pub struct AddedUser {
    pub name: String,
    pub uuid: String,
    pub vless_url: String,
}

/// Expand tilde in a PathBuf (e.g. `~/.ssh/id_ed25519` -> `/home/user/.ssh/id_ed25519`).
fn expand_key_path(path: Option<std::path::PathBuf>) -> Option<std::path::PathBuf> {
    path.map(|p| {
        let s = p.to_string_lossy();
        if s.starts_with("~/") {
            std::path::PathBuf::from(expand_tilde(&s))
        } else {
            p
        }
    })
}

/// Resolve SSH connection parameters from the app config.
fn resolve_connection(
    config: &Config,
) -> Result<(String, u16, String, Option<std::path::PathBuf>), AppError> {
    if let Some(ref alias) = config.ssh_host {
        match resolve_ssh_host(alias) {
            Some(sc) => {
                let host = sc.hostname.unwrap_or_else(|| alias.clone());
                let port = sc.port.unwrap_or(config.port);
                let user = sc.user.unwrap_or_else(|| config.user.clone());
                let key = expand_key_path(sc.identity_file.or_else(|| config.key_path.clone()));
                Ok((host, port, user, key))
            }
            None => {
                // Alias not found in ssh config, treat as hostname
                Ok((
                    alias.clone(),
                    config.port,
                    config.user.clone(),
                    expand_key_path(config.key_path.clone()),
                ))
            }
        }
    } else if let Some(ref host) = config.host {
        Ok((
            host.clone(),
            config.port,
            config.user.clone(),
            expand_key_path(config.key_path.clone()),
        ))
    } else {
        Err(AppError::Config("no host configured".to_string()))
    }
}

/// Connect to the server using the app config.
async fn connect(config: &Config) -> Result<SshSession, AppError> {
    let (hostname, port, user, key_path) = resolve_connection(config)?;
    let addr = format!("{}:{}", hostname, port);
    SshSession::connect(addr, &user, key_path.as_deref(), &config.container).await
}

/// Read the Xray public key from the server (needed for vless:// URL generation).
async fn read_public_key(session: &SshSession) -> Result<String, AppError> {
    let result = session
        .exec_command(&format!("cat {}", PUBLIC_KEY_PATH))
        .await?;
    if result.success() {
        Ok(result.stdout.trim().to_string())
    } else {
        Err(AppError::Xray(format!(
            "failed to read public key: {}",
            result.stderr.trim()
        )))
    }
}

/// Build a vless:// URL for a user, using live server config for reality params.
async fn build_vless_url(
    session: &SshSession,
    config: &Config,
    uuid: &str,
    name: &str,
) -> Result<String, AppError> {
    let server_config = read_server_config(session).await?;
    let reality = server_config
        .reality_settings()
        .ok_or_else(|| AppError::Xray("no Reality settings in server config".to_string()))?;
    let port = server_config.vless_port().unwrap_or(443);
    let public_key = read_public_key(session).await?;

    let (hostname, ..) = resolve_connection(config)?;

    let params = VlessUrlParams {
        uuid: uuid.to_string(),
        host: hostname,
        port,
        sni: reality.sni,
        public_key,
        short_id: reality.short_id,
        name: name.to_string(),
    };
    Ok(generate_vless_url(&params))
}

/// Spawn: fetch dashboard data (user list + server info + per-user stats).
pub fn spawn_fetch_dashboard(
    runtime: &tokio::runtime::Handle,
    config: Config,
    tx: mpsc::Sender<BackendMsg>,
) {
    runtime.spawn(async move {
        let result = fetch_dashboard_data(&config).await;
        let _ = tx.send(BackendMsg::DashboardData(result));
    });
}

async fn fetch_dashboard_data(config: &Config) -> Result<DashboardData, String> {
    let session = connect(config).await.map_err(|e| e.to_string())?;

    // Ensure the Xray API is enabled (adds api/stats/policy sections if missing).
    // This is idempotent — skips if already enabled.
    ensure_api_enabled(&session, &config.container)
        .await
        .map_err(|e| e.to_string())?;

    let client = XrayApiClient::new(&session);

    let mut users = client.list_users().await.map_err(|e| e.to_string())?;
    let server_info = client.get_server_info().await.map_err(|e| e.to_string())?;

    // Enrich users with stats and online count
    for user in &mut users {
        if let Ok(stats) = client.get_user_stats(&user.email).await {
            user.stats = stats;
        }
        if let Ok(count) = client.get_online_count(&user.email).await {
            user.online_count = count;
        }
    }

    let _ = session.close().await;

    Ok(DashboardData { users, server_info })
}

/// Spawn: test SSH connection and return xray version.
pub fn spawn_test_connection(
    runtime: &tokio::runtime::Handle,
    config: Config,
    tx: mpsc::Sender<BackendMsg>,
) {
    runtime.spawn(async move {
        let result = test_connection(&config).await;
        let _ = tx.send(BackendMsg::ConnectionTest(result));
    });
}

async fn test_connection(config: &Config) -> Result<String, String> {
    let session = connect(config).await.map_err(|e| e.to_string())?;
    let result = session
        .exec_in_container("xray version")
        .await
        .map_err(|e| e.to_string())?;
    let _ = session.close().await;

    if result.success() {
        Ok(result.stdout.trim().to_string())
    } else {
        Err(format!("xray version failed: {}", result.stderr.trim()))
    }
}

/// Spawn: add a new user.
pub fn spawn_add_user(
    runtime: &tokio::runtime::Handle,
    config: Config,
    name: String,
    tx: mpsc::Sender<BackendMsg>,
) {
    runtime.spawn(async move {
        let result = add_user(&config, &name).await;
        let _ = tx.send(BackendMsg::UserAdded(result));
    });
}

async fn add_user(config: &Config, name: &str) -> Result<AddedUser, String> {
    let session = connect(config).await.map_err(|e| e.to_string())?;
    let client = XrayApiClient::new(&session);

    let uuid = client.add_user(name).await.map_err(|e| e.to_string())?;

    // URL generation is best-effort: if it fails, the user was still added successfully.
    // The URL can be regenerated later via [c] or [q] in the detail view.
    let vless_url = match build_vless_url(&session, config, &uuid, name).await {
        Ok(url) => url,
        Err(e) => {
            eprintln!("warning: user added but URL generation failed: {}", e);
            String::new()
        }
    };

    let _ = session.close().await;

    Ok(AddedUser {
        name: name.to_string(),
        uuid,
        vless_url,
    })
}

/// Spawn: delete a user by UUID.
pub fn spawn_delete_user(
    runtime: &tokio::runtime::Handle,
    config: Config,
    uuid: String,
    tx: mpsc::Sender<BackendMsg>,
) {
    runtime.spawn(async move {
        let result = delete_user(&config, &uuid).await;
        let _ = tx.send(BackendMsg::UserDeleted(result));
    });
}

async fn delete_user(config: &Config, uuid: &str) -> Result<String, String> {
    let session = connect(config).await.map_err(|e| e.to_string())?;
    let client = XrayApiClient::new(&session);

    client.remove_user(uuid).await.map_err(|e| e.to_string())?;
    let _ = session.close().await;

    Ok(uuid.to_string())
}

/// Generate a vless:// URL for an existing user.
pub fn spawn_vless_url(
    runtime: &tokio::runtime::Handle,
    config: Config,
    uuid: String,
    name: String,
    intent: VlessUrlIntent,
    tx: mpsc::Sender<BackendMsg>,
) {
    runtime.spawn(async move {
        let result = generate_url(&config, &uuid, &name).await;
        let _ = tx.send(BackendMsg::VlessUrl(result, intent));
    });
}

/// Spawn: fetch online IPs for a user.
pub fn spawn_fetch_online_ips(
    runtime: &tokio::runtime::Handle,
    config: Config,
    uuid: String,
    email: String,
    tx: mpsc::Sender<BackendMsg>,
) {
    runtime.spawn(async move {
        let result = fetch_online_ips(&config, &uuid, &email).await;
        let _ = tx.send(BackendMsg::OnlineIps(result));
    });
}

async fn fetch_online_ips(
    config: &Config,
    uuid: &str,
    email: &str,
) -> Result<(String, Vec<String>), (String, String)> {
    let session = connect(config)
        .await
        .map_err(|e| (uuid.to_string(), e.to_string()))?;
    let client = XrayApiClient::new(&session);
    let ips = client
        .get_online_ips(email)
        .await
        .map_err(|e| (uuid.to_string(), e.to_string()))?;
    let _ = session.close().await;
    Ok((uuid.to_string(), ips))
}

async fn generate_url(config: &Config, uuid: &str, name: &str) -> Result<AddedUser, String> {
    let session = connect(config).await.map_err(|e| e.to_string())?;
    let vless_url = build_vless_url(&session, config, uuid, name)
        .await
        .map_err(|e| e.to_string())?;
    let _ = session.close().await;

    Ok(AddedUser {
        name: name.to_string(),
        uuid: uuid.to_string(),
        vless_url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_connection_with_host() {
        let config = Config {
            host: Some("1.2.3.4".to_string()),
            port: 2222,
            user: "admin".to_string(),
            key_path: None,
            ssh_host: None,
            container: "amnezia-xray".to_string(),
        };
        let (host, port, user, key) = resolve_connection(&config).unwrap();
        assert_eq!(host, "1.2.3.4");
        assert_eq!(port, 2222);
        assert_eq!(user, "admin");
        assert!(key.is_none());
    }

    #[test]
    fn test_resolve_connection_no_host() {
        let config = Config::default();
        let result = resolve_connection(&config);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_connection_ssh_host_not_in_config() {
        let config = Config {
            ssh_host: Some("nonexistent-alias".to_string()),
            host: None,
            port: 22,
            user: "root".to_string(),
            key_path: None,
            container: "amnezia-xray".to_string(),
        };
        // Falls back to treating alias as hostname
        let (host, port, user, _key) = resolve_connection(&config).unwrap();
        assert_eq!(host, "nonexistent-alias");
        assert_eq!(port, 22);
        assert_eq!(user, "root");
    }

    #[test]
    fn test_resolve_connection_expands_tilde_in_key_path() {
        let config = Config {
            host: Some("1.2.3.4".to_string()),
            port: 22,
            user: "root".to_string(),
            key_path: Some(std::path::PathBuf::from("~/.ssh/id_ed25519")),
            ssh_host: None,
            container: "amnezia-xray".to_string(),
        };
        let (_host, _port, _user, key) = resolve_connection(&config).unwrap();
        let key_path = key.expect("key_path should be Some");
        // Tilde should be expanded to the home directory
        assert!(
            !key_path.to_string_lossy().starts_with("~/"),
            "tilde should be expanded, got: {}",
            key_path.display()
        );
        assert!(
            key_path.to_string_lossy().ends_with(".ssh/id_ed25519"),
            "path suffix should be preserved, got: {}",
            key_path.display()
        );
    }

    #[test]
    fn test_resolve_connection_absolute_key_path_unchanged() {
        let config = Config {
            host: Some("1.2.3.4".to_string()),
            port: 22,
            user: "root".to_string(),
            key_path: Some(std::path::PathBuf::from("/home/user/.ssh/id_rsa")),
            ssh_host: None,
            container: "amnezia-xray".to_string(),
        };
        let (_host, _port, _user, key) = resolve_connection(&config).unwrap();
        let key_path = key.expect("key_path should be Some");
        assert_eq!(key_path.to_string_lossy(), "/home/user/.ssh/id_rsa");
    }

    #[test]
    fn test_expand_key_path_none() {
        assert!(expand_key_path(None).is_none());
    }
}
