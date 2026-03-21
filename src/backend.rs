//! Async backend operations for the TUI.
//!
//! Provides the bridge between the synchronous event loop and async SSH/Xray operations.
//! Operations are spawned on a tokio runtime and results are sent back via channels.

use std::sync::mpsc;

use crate::backend_trait::{SshBackend, XrayBackend};
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
    /// Telegram bot deployment result
    DeployBot(Result<String, String>),
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
pub fn resolve_connection_info(
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

/// Connect and return an `SshBackend` (trait-based abstraction).
pub async fn connect_backend(config: &Config) -> Result<SshBackend, AppError> {
    let (hostname, port, user, key_path) = resolve_connection_info(config)?;
    let addr = if hostname.contains(':') {
        format!("[{}]:{}", hostname, port)
    } else {
        format!("{}:{}", hostname, port)
    };
    let session = SshSession::connect(&addr, &user, key_path.as_deref(), &config.container).await?;
    Ok(SshBackend::new(session, hostname))
}

/// Read the Xray public key from the server (needed for vless:// URL generation).
async fn read_public_key(backend: &dyn XrayBackend) -> Result<String, AppError> {
    let result = backend
        .exec_in_container(&format!("cat {}", PUBLIC_KEY_PATH))
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
pub async fn build_vless_url(
    backend: &dyn XrayBackend,
    uuid: &str,
    name: &str,
) -> Result<String, AppError> {
    let server_config = read_server_config(backend).await?;
    let reality = server_config
        .reality_settings()
        .ok_or_else(|| AppError::Xray("no Reality settings in server config".to_string()))?;
    let port = server_config.vless_port().unwrap_or(443);
    let public_key = read_public_key(backend).await?;

    let params = VlessUrlParams {
        uuid: uuid.to_string(),
        host: backend.hostname().to_string(),
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
    api_check_done: bool,
) {
    runtime.spawn(async move {
        let result = fetch_dashboard_data(&config, api_check_done).await;
        let _ = tx.send(BackendMsg::DashboardData(result));
    });
}

async fn fetch_dashboard_data(
    config: &Config,
    api_check_done: bool,
) -> Result<DashboardData, String> {
    let backend = connect_backend(config).await.map_err(|e| e.to_string())?;

    // Ensure the Xray API is enabled (adds api/stats/policy sections if missing).
    // Only runs on the first successful refresh — skipped thereafter.
    if !api_check_done {
        ensure_api_enabled(&backend)
            .await
            .map_err(|e| e.to_string())?;
    }

    let client = XrayApiClient::new(&backend);

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

    let _ = backend.close().await;

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
    let backend = connect_backend(config).await.map_err(|e| e.to_string())?;
    let result = backend
        .exec_in_container("xray version")
        .await
        .map_err(|e| e.to_string())?;
    let _ = backend.close().await;

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
    let backend = connect_backend(config).await.map_err(|e| e.to_string())?;
    let client = XrayApiClient::new(&backend);

    let uuid = client.add_user(name).await.map_err(|e| e.to_string())?;

    // URL generation is best-effort: if it fails, the user was still added successfully.
    // The URL can be regenerated later via [c] or [q] in the detail view.
    let vless_url = match build_vless_url(&backend, &uuid, name).await {
        Ok(url) => url,
        Err(e) => {
            eprintln!("warning: user added but URL generation failed: {}", e);
            String::new()
        }
    };

    let _ = backend.close().await;

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
    let backend = connect_backend(config).await.map_err(|e| e.to_string())?;
    let client = XrayApiClient::new(&backend);

    client.remove_user(uuid).await.map_err(|e| e.to_string())?;
    let _ = backend.close().await;

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
    let backend = connect_backend(config)
        .await
        .map_err(|e| (uuid.to_string(), e.to_string()))?;
    let client = XrayApiClient::new(&backend);
    let ips = client
        .get_online_ips(email)
        .await
        .map_err(|e| (uuid.to_string(), e.to_string()))?;
    let _ = backend.close().await;
    Ok((uuid.to_string(), ips))
}

/// Spawn: deploy Telegram bot to VPS via SSH.
pub fn spawn_deploy_bot(
    runtime: &tokio::runtime::Handle,
    config: Config,
    token: String,
    tx: mpsc::Sender<BackendMsg>,
) {
    runtime.spawn(async move {
        let result = deploy_bot(&config, &token).await;
        let _ = tx.send(BackendMsg::DeployBot(result));
    });
}

pub async fn deploy_bot(config: &Config, token: &str) -> Result<String, String> {
    use base64::Engine;
    use crate::ui::telegram_setup::generate_compose_yaml;

    let backend = connect_backend(config).await.map_err(|e| e.to_string())?;

    // Create directory
    backend
        .exec_on_host("mkdir -p /opt/axadmin")
        .await
        .map_err(|e| format!("Failed to create directory: {}", e))?;

    // Write docker-compose.yml
    let compose = generate_compose_yaml(token, &config.container);
    let compose_b64 = base64::engine::general_purpose::STANDARD.encode(compose.as_bytes());
    let write_cmd = format!(
        "echo '{}' | base64 -d > /opt/axadmin/docker-compose.yml",
        compose_b64
    );
    let result = backend
        .exec_on_host(&write_cmd)
        .await
        .map_err(|e| format!("Failed to write compose file: {}", e))?;
    if !result.success() {
        return Err(format!(
            "Failed to write compose file: {}",
            result.stderr.trim()
        ));
    }
    // Restrict permissions — file contains the Telegram bot token
    backend
        .exec_on_host("chmod 600 /opt/axadmin/docker-compose.yml")
        .await
        .map_err(|e| format!("Failed to set compose file permissions: {}", e))?;

    // Write Dockerfile
    let dockerfile = include_str!("../Dockerfile");
    let dockerfile_b64 = base64::engine::general_purpose::STANDARD.encode(dockerfile.as_bytes());
    let write_df_cmd = format!(
        "echo '{}' | base64 -d > /opt/axadmin/Dockerfile",
        dockerfile_b64
    );
    let result = backend
        .exec_on_host(&write_df_cmd)
        .await
        .map_err(|e| format!("Failed to write Dockerfile: {}", e))?;
    if !result.success() {
        return Err(format!(
            "Failed to write Dockerfile: {}",
            result.stderr.trim()
        ));
    }

    // Upload source files for Docker build context
    for filename in &["Cargo.toml", "Cargo.lock"] {
        let content = std::fs::read_to_string(filename)
            .map_err(|e| format!("Failed to read {}: {} (run --deploy-bot from the repo directory)", filename, e))?;
        let content_b64 = base64::engine::general_purpose::STANDARD.encode(content.as_bytes());
        let cmd = format!(
            "echo '{}' | base64 -d > /opt/axadmin/{}",
            content_b64, filename
        );
        let result = backend
            .exec_on_host(&cmd)
            .await
            .map_err(|e| format!("Failed to upload {}: {}", filename, e))?;
        if !result.success() {
            return Err(format!("Failed to upload {}: {}", filename, result.stderr.trim()));
        }
    }

    // Upload src/ directory as tar archive
    let tar_output = tokio::process::Command::new("tar")
        .args(["cf", "-", "src/"])
        .output()
        .await
        .map_err(|e| format!("Failed to tar src/: {} (run --deploy-bot from the repo directory)", e))?;
    if !tar_output.status.success() {
        return Err("Failed to tar src/ directory. Run --deploy-bot from the repo directory.".to_string());
    }
    let tar_b64 = base64::engine::general_purpose::STANDARD.encode(&tar_output.stdout);
    let upload_src_cmd = format!(
        "echo '{}' | base64 -d | tar xf - -C /opt/axadmin/",
        tar_b64
    );
    let result = backend
        .exec_on_host(&upload_src_cmd)
        .await
        .map_err(|e| format!("Failed to upload src/: {}", e))?;
    if !result.success() {
        return Err(format!("Failed to upload src/: {}", result.stderr.trim()));
    }

    // Build image
    let result = backend
        .exec_on_host("cd /opt/axadmin && docker compose build 2>&1")
        .await
        .map_err(|e| format!("Docker build failed: {}", e))?;
    if !result.success() {
        return Err(format!("Docker build failed: {}", result.stderr.trim()));
    }

    // Stop existing and start
    let _ = backend
        .exec_on_host("cd /opt/axadmin && docker compose down 2>/dev/null")
        .await;
    let result = backend
        .exec_on_host("cd /opt/axadmin && docker compose up -d 2>&1")
        .await
        .map_err(|e| format!("Docker start failed: {}", e))?;
    if !result.success() {
        return Err(format!("Docker start failed: {}", result.stderr.trim()));
    }

    // Verify
    let result = backend
        .exec_on_host("cd /opt/axadmin && docker compose ps --format '{{.Status}}' 2>&1")
        .await
        .map_err(|e| format!("Verification failed: {}", e))?;

    let _ = backend.close().await;

    let status = result.stdout.trim().to_string();
    if status.contains("Up") || status.contains("running") {
        Ok("Bot deployed and running! Send /start to your bot.".to_string())
    } else {
        Ok(format!("Bot container created. Status: {}", status))
    }
}

async fn generate_url(config: &Config, uuid: &str, name: &str) -> Result<AddedUser, String> {
    let backend = connect_backend(config).await.map_err(|e| e.to_string())?;
    let vless_url = build_vless_url(&backend, uuid, name)
        .await
        .map_err(|e| e.to_string())?;
    let _ = backend.close().await;

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
            telegram_token: None,
            telegram_admin_chat_id: None,
        };
        let (host, port, user, key) = resolve_connection_info(&config).unwrap();
        assert_eq!(host, "1.2.3.4");
        assert_eq!(port, 2222);
        assert_eq!(user, "admin");
        assert!(key.is_none());
    }

    #[test]
    fn test_resolve_connection_no_host() {
        let config = Config::default();
        let result = resolve_connection_info(&config);
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
            telegram_token: None,
            telegram_admin_chat_id: None,
        };
        // Falls back to treating alias as hostname
        let (host, port, user, _key) = resolve_connection_info(&config).unwrap();
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
            telegram_token: None,
            telegram_admin_chat_id: None,
        };
        let (_host, _port, _user, key) = resolve_connection_info(&config).unwrap();
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
            telegram_token: None,
            telegram_admin_chat_id: None,
        };
        let (_host, _port, _user, key) = resolve_connection_info(&config).unwrap();
        let key_path = key.expect("key_path should be Some");
        assert_eq!(key_path.to_string_lossy(), "/home/user/.ssh/id_rsa");
    }

    #[test]
    fn test_expand_key_path_none() {
        assert!(expand_key_path(None).is_none());
    }

    #[test]
    fn test_public_key_path_is_inside_container() {
        // The public key lives inside the Amnezia container (no bind mounts).
        // read_public_key() must use exec_in_container(), not exec_command().
        assert_eq!(PUBLIC_KEY_PATH, "/opt/amnezia/xray/xray_public.key");
        assert!(PUBLIC_KEY_PATH.starts_with("/opt/amnezia/"));
    }
}
