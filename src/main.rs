mod backend_trait;
mod config;
mod error;
pub mod migrate;
mod ssh;
mod telegram;
mod xray;

use backend_trait::{LocalBackend, SshBackend, XrayBackend};
use clap::{CommandFactory, Parser};
use config::{Cli, Config};
use error::AppError;
use ssh::{expand_tilde, SshSession};
use std::io::IsTerminal;
use xray::client::XrayClient;

// ── Helpers inlined from the deleted src/backend.rs (Epic D Task 1.2) ──

/// GitHub API endpoint for the latest Xray-core release.
const XRAY_LATEST_RELEASE_URL: &str = "https://api.github.com/repos/XTLS/Xray-core/releases/latest";
/// Timeout for best-effort network calls (version check).
const NETWORK_PROBE_TIMEOUT_SECS: u32 = 3;

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

/// Fetch the latest Xray-core release tag from GitHub. `None` on network failure or
/// unparseable response. Returned string is the version without a leading `v` (e.g. `25.8.3`).
pub(crate) async fn fetch_latest_xray_version(backend: &dyn XrayBackend) -> Option<String> {
    let out = backend
        .exec_on_host(&format!(
            "curl -sfL --max-time {} -H 'Accept: application/vnd.github+json' {}",
            NETWORK_PROBE_TIMEOUT_SECS, XRAY_LATEST_RELEASE_URL
        ))
        .await
        .ok()?;
    if !out.success() {
        return None;
    }
    parse_xray_version_from_json(&out.stdout)
}

/// Extract and validate the Xray version from a GitHub releases/latest JSON payload.
fn parse_xray_version_from_json(body: &str) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    let tag = json.get("tag_name")?.as_str()?;
    let version = tag.strip_prefix('v').unwrap_or(tag);
    if version.is_empty()
        || !version.chars().all(|c| c.is_ascii_digit() || c == '.')
        || !version.chars().any(|c| c.is_ascii_digit())
    {
        return None;
    }
    Some(version.to_string())
}

// ── Connection helpers ──

/// Build a `LocalBackend` using the configured host (or "localhost" fallback).
fn connect_local(config: &Config) -> LocalBackend {
    let hostname = config
        .host
        .clone()
        .unwrap_or_else(|| "localhost".to_string());
    LocalBackend::new(hostname)
}

/// Open an SSH session and wrap it in an `SshBackend`.
async fn connect_ssh(config: &Config) -> Result<SshBackend, AppError> {
    let hostname = config
        .host
        .clone()
        .ok_or_else(|| AppError::Config("missing --host".to_string()))?;
    let port = config.port;
    let user = &config.user;
    let key_path = expand_key_path(config.key_path.clone());
    let addr = if hostname.contains(':') {
        format!("[{}]:{}", hostname, port)
    } else {
        format!("{}:{}", hostname, port)
    };
    // No Docker container in the bridge-native world; pass "" through.
    let session = SshSession::connect(&addr, user, key_path.as_deref(), "").await?;
    Ok(SshBackend::new(session, hostname))
}

/// Unified backend constructor: `LocalBackend` when `--local`, `SshBackend` otherwise.
async fn connect(config: &Config, cli: &Cli) -> error::Result<Box<dyn XrayBackend>> {
    if cli.local {
        Ok(Box::new(connect_local(config)))
    } else {
        Ok(Box::new(connect_ssh(config).await?))
    }
}

fn main() {
    let cli = Cli::parse();

    let mut config = match Config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Warning: failed to load config: {}", e);
            Config::default()
        }
    };

    config.merge_cli(&cli);

    // Create tokio runtime for async SSH/Xray operations
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("Failed to create async runtime: {}", e);
            std::process::exit(1);
        }
    };

    // Non-interactive CLI commands
    if cli.list_users {
        if let Err(e) = runtime.block_on(cli_list_users(&config, &cli)) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    if let Some(ref name) = cli.user_url {
        if let Err(e) = runtime.block_on(cli_user_url(&config, &cli, name)) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    if let Some(ref name) = cli.user_qr {
        if let Err(e) = runtime.block_on(cli_user_qr(&config, &cli, name)) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    if cli.online_status {
        if let Err(e) = runtime.block_on(cli_online_status(&config, &cli)) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    if cli.server_info {
        if let Err(e) = runtime.block_on(cli_server_info(&config, &cli)) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    if cli.telegram_bot {
        if !cli.local {
            eprintln!(
                "Error: --telegram-bot requires --local flag (must run on the VPS, not over SSH)"
            );
            std::process::exit(1);
        }
        let token = match cli
            .telegram_token
            .clone()
            .or_else(|| config.telegram_token.clone())
        {
            Some(t) => t,
            None => {
                eprintln!("Error: --telegram-token or TELEGRAM_TOKEN env var is required");
                std::process::exit(1);
            }
        };
        if config.telegram_admin_chat_id.is_none() {
            eprintln!("Error: Admin ID required. Use --admin-id <your_telegram_id> or set ADMIN_ID env var.");
            eprintln!("To find your Telegram ID, send /start to @userinfobot.");
            std::process::exit(1);
        }
        if let Err(e) = runtime.block_on(cli_telegram_bot(&config, &cli, &token)) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    if let Some(ref name) = cli.add_user {
        if let Err(e) = runtime.block_on(cli_add_user(&config, &cli, name)) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    if let Some(ref name) = cli.delete_user {
        if let Err(e) = runtime.block_on(cli_delete_user(&config, &cli, name, cli.yes)) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    // No CLI subcommand matched — print help and exit non-zero.
    Cli::command().print_help().ok();
    std::process::exit(1);
}

// ── CLI handlers (bridge-mode only) ──

async fn cli_list_users(config: &Config, cli: &Cli) -> error::Result<()> {
    let backend = connect(config, cli).await?;
    let client = XrayClient::new(backend.as_ref());
    let clients = client.list_clients().await?;

    if clients.is_empty() {
        println!("No users found.");
        return Ok(());
    }

    println!("{:<30} {:<36}", "NAME", "UUID");
    println!("{}", "-".repeat(68));
    for c in &clients {
        let name = c.email.strip_suffix("@vpn").unwrap_or(&c.email);
        println!("{:<30} {:<36}", name, c.uuid);
    }

    Ok(())
}

async fn cli_user_url(config: &Config, cli: &Cli, name: &str) -> error::Result<()> {
    let backend = connect(config, cli).await?;
    let client = XrayClient::new(backend.as_ref());
    let uuid = client.get_uuid(name).await?;
    let params = client.bridge_public_params().await?;

    let host = cli
        .host
        .clone()
        .unwrap_or_else(|| backend.hostname().to_string());
    let url = crate::xray::url::render_xhttp_url(&crate::xray::url::XhttpUrlParams {
        uuid,
        host,
        port: params.port,
        path: params.path,
        sni: params.sni,
        public_key: params.public_key,
        short_id: params.short_id,
        name: name.to_string(),
    });
    println!("{}", url);
    Ok(())
}

async fn cli_user_qr(config: &Config, cli: &Cli, name: &str) -> error::Result<()> {
    let backend = connect(config, cli).await?;
    let client = XrayClient::new(backend.as_ref());
    let uuid = client.get_uuid(name).await?;
    let params = client.bridge_public_params().await?;

    let host = cli
        .host
        .clone()
        .unwrap_or_else(|| backend.hostname().to_string());
    let url = crate::xray::url::render_xhttp_url(&crate::xray::url::XhttpUrlParams {
        uuid,
        host,
        port: params.port,
        path: params.path,
        sni: params.sni,
        public_key: params.public_key,
        short_id: params.short_id,
        name: name.to_string(),
    });
    println!("{}", name);
    println!("{}", url);
    println!();
    println!("{}", crate::xray::url::render_qr_ascii(&url));
    Ok(())
}

async fn cli_online_status(config: &Config, cli: &Cli) -> error::Result<()> {
    // The native-xray bridge does not expose the stats API that backed the
    // legacy `--online-status` table. Listing users with their UUIDs is the
    // most useful thing we can still surface.
    let backend = connect(config, cli).await?;
    let client = XrayClient::new(backend.as_ref());
    let clients = client.list_clients().await?;

    if clients.is_empty() {
        println!("No users found.");
        return Ok(());
    }

    println!("Online-status API is not available on the native bridge.");
    println!("Users currently configured:");
    for c in &clients {
        let name = c.email.strip_suffix("@vpn").unwrap_or(&c.email);
        println!("  - {} ({})", name, c.uuid);
    }

    Ok(())
}

async fn cli_server_info(config: &Config, cli: &Cli) -> error::Result<()> {
    let backend = connect(config, cli).await?;
    let client = XrayClient::new(backend.as_ref());
    let clients = client.list_clients().await?;
    let params = client.bridge_public_params().await?;

    let latest_version = fetch_latest_xray_version(backend.as_ref()).await;
    let version_display = match &latest_version {
        Some(latest) => format!("upstream v{}", latest),
        None => "upstream version: unknown".to_string(),
    };

    println!("Backend:      native systemd xray (bridge)");
    println!("Host:         {}", backend.hostname());
    println!("Port:         {}", params.port);
    println!("SNI:          {}", params.sni);
    println!("Path:         {}", params.path);
    println!("Users:        {}", clients.len());
    println!("Xray:         {}", version_display);

    Ok(())
}

async fn cli_telegram_bot(config: &Config, cli: &Cli, token: &str) -> error::Result<()> {
    env_logger::init();
    let backend = connect(config, cli).await?;
    telegram::run_bot(token, backend, config.clone()).await
}

async fn cli_add_user(config: &Config, cli: &Cli, name: &str) -> error::Result<()> {
    if name.trim().is_empty() {
        return Err(error::AppError::Xray(
            "user name cannot be empty".to_string(),
        ));
    }

    let backend = connect(config, cli).await?;
    let client = XrayClient::new(backend.as_ref());
    let entry = client.add_client(name).await?;
    let params = client.bridge_public_params().await?;

    let host = cli
        .host
        .clone()
        .unwrap_or_else(|| backend.hostname().to_string());
    let url = crate::xray::url::render_xhttp_url(&crate::xray::url::XhttpUrlParams {
        uuid: entry.uuid.clone(),
        host,
        port: params.port,
        path: params.path,
        sni: params.sni,
        public_key: params.public_key,
        short_id: params.short_id,
        name: name.to_string(),
    });

    client.reload_xray().await?;

    println!("User added successfully.");
    println!("Name:  {}", name);
    println!("UUID:  {}", entry.uuid);
    println!("URL:   {}", url);
    println!();
    println!("{}", crate::xray::url::render_qr_ascii(&url));

    Ok(())
}

async fn cli_delete_user(
    config: &Config,
    cli: &Cli,
    name: &str,
    yes: bool,
) -> error::Result<()> {
    let backend = connect(config, cli).await?;
    let client = XrayClient::new(backend.as_ref());

    // Verify user exists up front so we can fail fast with a clear message.
    let _ = client.get_uuid(name).await?;

    if !yes {
        if !std::io::stdin().is_terminal() {
            return Err(error::AppError::Config(
                "Interactive confirmation required. Use --yes to skip.".to_string(),
            ));
        }
        eprintln!(
            "Are you sure you want to delete user '{}'? Type the user name to confirm:",
            name
        );
        let mut input = String::new();
        std::io::stdin()
            .read_line(&mut input)
            .map_err(error::AppError::Io)?;
        if input.trim() != name {
            eprintln!("Name does not match. Deletion cancelled.");
            return Ok(());
        }
    }

    client.remove_client(name).await?;
    client.reload_xray().await?;
    println!("User '{}' deleted.", name);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_key_path_none() {
        assert!(expand_key_path(None).is_none());
    }

    #[test]
    fn test_expand_key_path_tilde() {
        let p = expand_key_path(Some(std::path::PathBuf::from("~/.ssh/id_ed25519")));
        let p = p.expect("should be Some");
        assert!(!p.to_string_lossy().starts_with("~/"));
        assert!(p.to_string_lossy().ends_with(".ssh/id_ed25519"));
    }

    #[test]
    fn test_expand_key_path_absolute_unchanged() {
        let p = expand_key_path(Some(std::path::PathBuf::from("/home/user/.ssh/id_rsa")));
        assert_eq!(p.unwrap().to_string_lossy(), "/home/user/.ssh/id_rsa");
    }

    #[test]
    fn test_parse_xray_version_valid() {
        let body = r#"{"tag_name":"v25.8.3","name":"v25.8.3"}"#;
        assert_eq!(parse_xray_version_from_json(body), Some("25.8.3".into()));
    }

    #[test]
    fn test_parse_xray_version_no_v_prefix() {
        let body = r#"{"tag_name":"25.8.3"}"#;
        assert_eq!(parse_xray_version_from_json(body), Some("25.8.3".into()));
    }

    #[test]
    fn test_parse_xray_version_missing_field() {
        let body = r#"{"name":"v25.8.3"}"#;
        assert_eq!(parse_xray_version_from_json(body), None);
    }

    #[test]
    fn test_parse_xray_version_malformed_json() {
        assert_eq!(parse_xray_version_from_json("not json"), None);
        assert_eq!(parse_xray_version_from_json(""), None);
    }

    #[test]
    fn test_parse_xray_version_rejects_non_semver() {
        let body = r#"{"tag_name":"rate limit exceeded"}"#;
        assert_eq!(parse_xray_version_from_json(body), None);
        let body = r#"{"tag_name":"v1.0-beta"}"#;
        assert_eq!(parse_xray_version_from_json(body), None);
        let body = r#"{"tag_name":"v"}"#;
        assert_eq!(parse_xray_version_from_json(body), None);
        let body = r#"{"tag_name":""}"#;
        assert_eq!(parse_xray_version_from_json(body), None);
    }
}
