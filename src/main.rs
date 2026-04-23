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
use ssh::{expand_tilde, resolve_ssh_host, SshSession};
use std::io::IsTerminal;
use xray::types::VlessUrlParams;

// ── Helpers inlined from the deleted src/backend.rs (Epic D Task 1.2) ──
//
// Previously these lived alongside TUI task spawners; the TUI is gone,
// but a handful of helpers are still called from CLI paths in this file
// and from `src/telegram.rs`. They're kept `pub(crate)` so `telegram.rs`
// can continue to reference them until Phase 4/Task 5.1 rewires bot code
// to the new bridge XrayClient.

/// Path to the Xray public key file on the server (used for vless:// URLs).
const PUBLIC_KEY_PATH: &str = "/opt/amnezia/xray/xray_public.key";

/// GitHub API endpoint for the latest Xray-core release.
const XRAY_LATEST_RELEASE_URL: &str = "https://api.github.com/repos/XTLS/Xray-core/releases/latest";
/// Timeout for best-effort network calls (uptime probe, version check).
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

/// Resolve SSH connection parameters from the app config.
fn resolve_connection_info(
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
async fn connect_backend(config: &Config) -> Result<SshBackend, AppError> {
    let (hostname, port, user, key_path) = resolve_connection_info(config)?;
    let addr = if hostname.contains(':') {
        format!("[{}]:{}", hostname, port)
    } else {
        format!("{}:{}", hostname, port)
    };
    let session = SshSession::connect(&addr, &user, key_path.as_deref(), &config.container).await?;
    Ok(SshBackend::new(session, hostname))
}

/// Fetch `docker ps` status for the backend's container. Empty string on failure or no match.
///
/// Uses an anchored regex filter so that `amnezia-xray-test` does not match `amnezia-xray`.
/// Container names are validated (`is_valid_container_name`) to safe characters so direct
/// interpolation is fine.
pub(crate) async fn fetch_container_uptime(backend: &dyn XrayBackend) -> String {
    backend
        .exec_on_host(&format!(
            "docker ps --filter 'name=^/{}$' --format '{{{{.Status}}}}'",
            backend.container_name()
        ))
        .await
        .map(|o| o.stdout.trim().to_string())
        .unwrap_or_default()
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

/// Read the Xray public key from the server (needed for vless:// URL generation).
async fn read_public_key(backend: &dyn XrayBackend) -> Result<String, AppError> {
    let result = backend
        .exec_in_container(&format!("cat {}", PUBLIC_KEY_PATH))
        .await?;
    if result.success() {
        Ok(result.stdout.trim().to_string())
    } else {
        let msg = format!("failed to read public key: {}", result.stderr.trim());
        Err(AppError::Xray(crate::error::add_hint(&msg)))
    }
}

/// Build a vless:// URL for a user — legacy Amnezia path (reads server.json from
/// the container to pull Reality params).
///
/// Kept `pub(crate)` so `src/telegram.rs` still compiles in its legacy
/// non-bridge branches until Task 5.1 rips those out. The body no longer
/// works because `xray::config::read_server_config` was deleted in Task 3.2;
/// returning an error here is fine because every caller is scheduled for
/// removal in Task 5.1 / 4.2.
pub(crate) async fn build_vless_url(
    _backend: &dyn XrayBackend,
    _uuid: &str,
    _name: &str,
) -> Result<String, AppError> {
    let _ = VlessUrlParams {
        uuid: String::new(),
        host: String::new(),
        port: 0,
        sni: String::new(),
        public_key: String::new(),
        short_id: String::new(),
        name: String::new(),
    };
    Err(AppError::Xray(
        "legacy Amnezia vless URL builder removed (Task 3.2); bridge path uses xray::url instead"
            .to_string(),
    ))
}

/// Get VPS public IP. Tries curl on the host first, falls back to SSH config host.
async fn get_vps_public_ip(backend: &SshBackend, config: &Config) -> String {
    // Try external service via SSH on the host
    for url in &[
        "https://ifconfig.me",
        "https://icanhazip.com",
        "https://api.ipify.org",
    ] {
        if let Ok(result) = backend
            .exec_on_host(&format!("curl -sf --max-time 5 {}", url))
            .await
        {
            if result.success() {
                let ip = result.stdout.trim().to_string();
                if !ip.is_empty() && ip.parse::<std::net::IpAddr>().is_ok() {
                    return ip;
                }
            }
        }
    }
    // Fallback: resolve from SSH config (host or ssh_host alias)
    if let Ok((hostname, ..)) = resolve_connection_info(config) {
        return hostname;
    }
    config
        .host
        .clone()
        .or_else(|| config.ssh_host.clone())
        .unwrap_or_else(|| "UNKNOWN_IP".to_string())
}

/// Deploy the Telegram bot to the VPS via SSH.
async fn deploy_bot(config: &Config, token: &str) -> Result<String, String> {
    let admin_id = config
        .telegram_admin_chat_id
        .ok_or_else(|| "admin_id is required for deploy".to_string())?;

    let backend = connect_backend(config).await.map_err(|e| e.to_string())?;

    // Pull pre-built Docker image from GitHub Container Registry
    let image = &config.bot_image;
    let result = backend
        .exec_on_host(&format!("docker pull {} 2>&1", image))
        .await
        .map_err(|e| format!("Docker pull failed: {}", e))?;
    if !result.success() {
        return Err(format!("Docker pull failed: {}", result.combined_output()));
    }

    // Stop existing container if running
    let _ = backend
        .exec_on_host("docker stop axadmin 2>/dev/null; docker rm axadmin 2>/dev/null")
        .await;

    // Get VPS public IP so the bot can generate correct vless:// URLs
    let vps_ip = get_vps_public_ip(&backend, config).await;

    let run_cmd = format!(
        "docker run -d --name axadmin --restart unless-stopped \
         -v /var/run/docker.sock:/var/run/docker.sock \
         -e TELEGRAM_TOKEN='{}' \
         -e ADMIN_ID='{}' \
         -e XRAY_CONTAINER='{}' \
         {} \
         --telegram-bot --local --container '{}' --admin-id {} --host '{}' 2>&1",
        token, admin_id, config.container, image, config.container, admin_id, vps_ip
    );
    let result = backend
        .exec_on_host(&run_cmd)
        .await
        .map_err(|e| format!("Docker start failed: {}", e))?;
    if !result.success() {
        return Err(format!("Docker start failed: {}", result.combined_output()));
    }

    // Verify container is running
    let result = backend
        .exec_on_host("docker inspect axadmin --format '{{.State.Status}}' 2>&1")
        .await
        .map_err(|e| format!("Verification failed: {}", e))?;

    let _ = backend.close().await;

    if !result.success() {
        return Err(format!("Verification failed: {}", result.combined_output()));
    }

    let status = result.stdout.trim().to_string();
    if status.contains("Up") || status.contains("running") {
        Ok("Bot deployed and running! Send /start to your bot.".to_string())
    } else {
        Err(format!("Bot container not running. Status: {}", status))
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

    let local = cli.local;

    // Non-interactive CLI commands
    if cli.list_users {
        if let Err(e) = runtime.block_on(cli_list_users(&config, local)) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    if let Some(ref name) = cli.user_url {
        if let Err(e) = runtime.block_on(cli_user_url(&config, name, local)) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    if let Some(ref name) = cli.user_qr {
        if let Err(e) = runtime.block_on(cli_user_qr(&config, name, local)) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    if cli.online_status {
        if let Err(e) = runtime.block_on(cli_online_status(&config, local)) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    if cli.server_info {
        if let Err(e) = runtime.block_on(cli_server_info(&config, local)) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    if cli.telegram_bot {
        if !local {
            eprintln!(
                "Error: --telegram-bot requires --local flag (must run on the VPS, not over SSH)"
            );
            eprintln!(
                "Deploy the bot to VPS with: cargo run -- --deploy-bot --telegram-token <TOKEN>"
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
        if let Err(e) = runtime.block_on(cli_telegram_bot(&config, &token, local, cli.bridge)) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    if cli.deploy_bot {
        let token = match cli
            .telegram_token
            .clone()
            .or_else(|| config.telegram_token.clone())
        {
            Some(t) => t,
            None => {
                eprintln!("Error: --telegram-token or TELEGRAM_TOKEN env var is required for --deploy-bot");
                std::process::exit(1);
            }
        };
        if config.telegram_admin_chat_id.is_none() {
            eprintln!("Error: --admin-id is required for --deploy-bot.");
            eprintln!("To find your Telegram ID, send /start to @userinfobot.");
            std::process::exit(1);
        }
        if let Err(e) = runtime.block_on(cli_deploy_bot(&config, &token)) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    if let Some(ref name) = cli.add_user {
        if let Err(e) = runtime.block_on(cli_add_user(&config, name, local, cli.bridge)) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    if let Some(ref name) = cli.delete_user {
        if let Err(e) = runtime.block_on(cli_delete_user(&config, name, local, cli.yes)) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    if let Some(ref names) = cli.rename_user {
        if let Err(e) = runtime.block_on(cli_rename_user(&config, &names[0], &names[1], local)) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    // No CLI subcommand matched — print help and exit non-zero.
    Cli::command().print_help().ok();
    std::process::exit(1);
}

/// Create a backend for CLI commands: either LocalBackend (--local) or SshBackend (default).
async fn connect_cli_backend(config: &Config, local: bool) -> error::Result<Box<dyn XrayBackend>> {
    if local {
        // Use --host if provided (e.g. from deploy), otherwise auto-detect
        let hostname = if let Some(ref host) = config.host {
            host.clone()
        } else {
            get_local_hostname().await
        };
        Ok(Box::new(LocalBackend::new(hostname)))
    } else {
        let backend = connect_backend(config).await?;
        Ok(Box::new(backend))
    }
}

/// Get the server's public IP for vless URL generation.
///
/// In --local mode (especially inside Docker), `hostname -I` returns the
/// container's private IP, which is unusable for VPN clients. Instead, query
/// an external service for the real public IP first.
async fn get_local_hostname() -> String {
    // Try to get the public IP via external service (works from inside Docker)
    for url in &[
        "https://ifconfig.me",
        "https://icanhazip.com",
        "https://api.ipify.org",
    ] {
        if let Ok(output) = tokio::process::Command::new("curl")
            .args(["-sf", "--max-time", "5", url])
            .output()
            .await
        {
            if output.status.success() {
                let ip = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !ip.is_empty() && looks_like_ip(&ip) {
                    return ip;
                }
            }
        }
    }
    // Fallback to hostname -I (useful when running directly on VPS without internet issues)
    if let Ok(output) = tokio::process::Command::new("hostname")
        .arg("-I")
        .output()
        .await
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Some(ip) = stdout.split_whitespace().next() {
            return ip.to_string();
        }
    }
    // All methods failed — return a placeholder that signals the issue
    eprintln!("Warning: could not determine server IP address. VPN URLs may be unusable.");
    "UNKNOWN_IP".to_string()
}

/// Basic check that a string looks like an IPv4 or IPv6 address.
fn looks_like_ip(s: &str) -> bool {
    // Must not contain HTML or spaces (rate-limit pages, error messages)
    if s.contains('<') || s.contains(' ') || s.len() > 45 {
        return false;
    }
    s.parse::<std::net::IpAddr>().is_ok()
}

async fn cli_user_url(config: &Config, name: &str, local: bool) -> error::Result<()> {
    let backend = connect_cli_backend(config, local).await?;
    let client = xray::client::XrayApiClient::new(backend.as_ref());
    let users = client.list_users().await?;

    let user = users.iter().find(|u| u.name == name);
    let user = match user {
        Some(u) => u,
        None => {
            return Err(error::AppError::Xray(format!("user '{}' not found", name)));
        }
    };

    // TODO(Epic D Phase 4): rewire to bridge URL
    let _ = user;
    let vless_url = String::new();

    println!("{}", vless_url);
    Ok(())
}

async fn cli_user_qr(config: &Config, name: &str, local: bool) -> error::Result<()> {
    let backend = connect_cli_backend(config, local).await?;
    let client = xray::client::XrayApiClient::new(backend.as_ref());
    let users = client.list_users().await?;

    let user = users.iter().find(|u| u.name == name);
    let user = match user {
        Some(u) => u,
        None => {
            return Err(error::AppError::Xray(format!("user '{}' not found", name)));
        }
    };

    // TODO(Epic D Phase 4): rewire to bridge URL
    let _ = user;
    let vless_url = String::new();

    // TODO(Epic D Task 2.x): re-wire QR rendering after ui::qr is relocated.
    println!("{}", name);
    println!("{}", vless_url);

    Ok(())
}

async fn cli_online_status(config: &Config, local: bool) -> error::Result<()> {
    let backend = connect_cli_backend(config, local).await?;
    let client = xray::client::XrayApiClient::new(backend.as_ref());
    let users = client.list_users().await?;

    if users.is_empty() {
        println!("No users found.");
        return Ok(());
    }

    // Collect online status for each user
    let mut rows: Vec<(String, u32, Vec<String>)> = Vec::new();
    for user in &users {
        let count = client.get_online_count(&user.email).await.unwrap_or(0);
        let ips = if count > 0 {
            client.get_online_ips(&user.email).await.unwrap_or_default()
        } else {
            Vec::new()
        };
        let name = if user.name.is_empty() {
            user.uuid[..std::cmp::min(8, user.uuid.len())].to_string()
        } else {
            user.name.clone()
        };
        rows.push((name, count, ips));
    }

    // Print table
    println!("{:<30} {:<8} IPs", "NAME", "ONLINE");
    println!("{}", "-".repeat(60));

    for (name, count, ips) in &rows {
        let online = if *count > 0 {
            format!("● {}", count)
        } else {
            "○".to_string()
        };
        let ip_str = if ips.is_empty() {
            "-".to_string()
        } else {
            ips.join(", ")
        };
        println!("{:<30} {:<8} {}", name, online, ip_str);
    }

    Ok(())
}

async fn cli_server_info(config: &Config, local: bool) -> error::Result<()> {
    let backend = connect_cli_backend(config, local).await?;
    let client = xray::client::XrayApiClient::new(backend.as_ref());

    let server_info = client.get_server_info().await?;
    let users = client.list_users().await?;
    // Check API status by probing the stats endpoint — `xray version` works
    // without the API, so a successful version alone does not prove the
    // runtime API is healthy.  `probe_stats_api` checks the command exit code
    // instead of silently falling back to zero.
    let api_ok = client.probe_stats_api().await.unwrap_or(false);
    let api_status = if api_ok {
        "enabled"
    } else if server_info.version != "unknown" {
        "degraded (version ok, stats API unreachable)"
    } else {
        "unknown"
    };

    // Get container uptime and check for newer Xray via shared helpers
    // (keeps the 3 s GitHub timeout and anchored docker filter consistent with the TUI/Telegram).
    let uptime_raw = fetch_container_uptime(backend.as_ref()).await;
    let uptime = if uptime_raw.is_empty() {
        "unknown".to_string()
    } else {
        uptime_raw
    };
    let latest_version = fetch_latest_xray_version(backend.as_ref()).await;

    let version_display = match &latest_version {
        Some(latest) if latest != &server_info.version => {
            format!("v{} (update available: v{})", server_info.version, latest)
        }
        Some(_) => format!("v{} (latest)", server_info.version),
        None => format!("v{}", server_info.version),
    };

    println!("Xray:           {}", version_display);
    println!("API status:     {}", api_status);
    println!("Uptime:         {}", uptime);
    println!("Users:          {}", users.len());
    // TODO(Epic D Task 2.x): re-introduce human-readable byte formatting.
    println!("Total upload:   {} B", server_info.uplink);
    println!("Total download: {} B", server_info.downlink);

    Ok(())
}

async fn cli_list_users(config: &Config, local: bool) -> error::Result<()> {
    if !local {
        let (_, port, user, _) = resolve_connection_info(config)?;
        eprintln!(
            "Connecting to {}@{}:{}...",
            user,
            config
                .ssh_host
                .as_deref()
                .or(config.host.as_deref())
                .unwrap_or("?"),
            port
        );
    }

    let backend = connect_cli_backend(config, local).await?;
    let client = xray::client::XrayApiClient::new(backend.as_ref());
    let users = client.list_users().await?;

    // Fetch stats for each user
    let mut users_with_stats = Vec::new();
    for mut user in users {
        if let Ok(stats) = client.get_user_stats(&user.email).await {
            user.stats = stats;
        }
        if let Ok(count) = client.get_online_count(&user.email).await {
            user.online_count = count;
        }
        users_with_stats.push(user);
    }

    if users_with_stats.is_empty() {
        println!("No users found.");
        return Ok(());
    }

    // Print header
    println!(
        "{:<30} {:<10} {:<12} {:<12} {:<8}",
        "NAME", "UUID", "UPLOAD", "DOWNLOAD", "ONLINE"
    );
    println!("{}", "-".repeat(72));

    for user in &users_with_stats {
        let name = if user.name.is_empty() {
            &user.uuid[..std::cmp::min(8, user.uuid.len())]
        } else {
            &user.name
        };
        let uuid_short = &user.uuid[..std::cmp::min(8, user.uuid.len())];
        let online = if user.online_count > 0 {
            format!("● {}", user.online_count)
        } else {
            "○".to_string()
        };

        // TODO(Epic D Task 2.x): re-introduce human-readable byte formatting.
        println!(
            "{:<30} {:<10} {:<12} {:<12} {:<8}",
            name,
            uuid_short,
            format!("{} B", user.stats.uplink),
            format!("{} B", user.stats.downlink),
            online,
        );
    }

    Ok(())
}

async fn cli_telegram_bot(
    config: &Config,
    token: &str,
    local: bool,
    bridge: bool,
) -> error::Result<()> {
    env_logger::init();

    if bridge {
        // Bridge mode: native systemd xray, no Amnezia Docker container.
        let backend: Box<dyn XrayBackend> = if local {
            let hostname = if let Some(ref host) = config.host {
                host.clone()
            } else {
                get_local_hostname().await
            };
            Box::new(LocalBackend::new(hostname))
        } else {
            let (hostname, port, user, key_path) = resolve_connection_info(config)?;
            let addr = if hostname.contains(':') {
                format!("[{}]:{}", hostname, port)
            } else {
                format!("{}:{}", hostname, port)
            };
            let session =
                ssh::SshSession::connect(&addr, &user, key_path.as_deref(), &config.container)
                    .await?;
            Box::new(SshBackend::new(session, hostname))
        };
        return telegram::run_bot(token, backend, config.clone(), true).await;
    }

    let backend = connect_cli_backend(config, local).await?;

    // Legacy (non-bridge) bot path: the Amnezia-Docker `ensure_api_enabled`
    // bootstrap was removed in Task 3.2. The bot's legacy branches in
    // `telegram.rs` are scheduled for removal in Task 5.1; until then this
    // path will fail at runtime when any API-backed command is invoked.
    telegram::run_bot(token, backend, config.clone(), false).await
}

async fn cli_add_user(config: &Config, name: &str, local: bool, bridge: bool) -> error::Result<()> {
    if name.trim().is_empty() {
        return Err(error::AppError::Xray(
            "user name cannot be empty".to_string(),
        ));
    }

    if bridge {
        // Native bridge: no Amnezia API bootstrap, no clientsTable.
        let backend: Box<dyn XrayBackend> = if local {
            let hostname = if let Some(ref host) = config.host {
                host.clone()
            } else {
                get_local_hostname().await
            };
            Box::new(LocalBackend::new(hostname))
        } else {
            let (hostname, port, user, key_path) = resolve_connection_info(config)?;
            let addr = if hostname.contains(':') {
                format!("[{}]:{}", hostname, port)
            } else {
                format!("{}:{}", hostname, port)
            };
            let session =
                ssh::SshSession::connect(&addr, &user, key_path.as_deref(), &config.container)
                    .await?;
            Box::new(SshBackend::new(session, hostname))
        };

        let client = xray::client::XrayClient::new(backend.as_ref());
        let entry = client.add_client(name).await?;

        println!("User added successfully.");
        println!("Name:  {}", name);
        println!("UUID:  {}", entry.uuid);

        match client.bridge_public_params().await {
            Ok(params) => {
                let url = xray::url::render_xhttp_url(&xray::url::XhttpUrlParams {
                    uuid: entry.uuid.clone(),
                    host: backend.hostname().to_string(),
                    port: params.port,
                    path: params.path,
                    sni: params.sni,
                    public_key: params.public_key,
                    short_id: params.short_id,
                    name: name.to_string(),
                });
                println!("URL:   {}", url);
                println!();
                println!("{}", xray::url::render_qr_ascii(&url));
            }
            Err(e) => eprintln!(
                "Warning: URL generation failed: {}. Use --user-url to retry.",
                e
            ),
        }
        return Ok(());
    }

    let backend = connect_cli_backend(config, local).await?;

    // Legacy (non-bridge) add-user path: the Amnezia-Docker pre-check + API
    // bootstrap (`ensure_api_enabled`, `read_server_config`, `read_clients_table`)
    // were removed in Task 3.2. Full removal of this branch happens in Task 4.2.
    let client = xray::client::XrayApiClient::new(backend.as_ref());

    let uuid = client.add_user(name).await?;

    println!("User added successfully.");
    println!("Name:  {}", name);
    println!("UUID:  {}", uuid);

    // TODO(Epic D Phase 4): rewire to bridge URL
    let _ = uuid;

    Ok(())
}

async fn cli_delete_user(config: &Config, name: &str, local: bool, yes: bool) -> error::Result<()> {
    let backend = connect_cli_backend(config, local).await?;
    let client = xray::client::XrayApiClient::new(backend.as_ref());
    let users = client.list_users().await?;

    let user = users.iter().find(|u| u.name == name);
    let user = match user {
        Some(u) => u,
        None => {
            return Err(error::AppError::Xray(format!("user '{}' not found", name)));
        }
    };

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

    client.remove_user(&user.uuid).await?;
    println!("User '{}' deleted.", name);

    Ok(())
}

async fn cli_rename_user(
    config: &Config,
    old_name: &str,
    new_name: &str,
    local: bool,
) -> error::Result<()> {
    let backend = connect_cli_backend(config, local).await?;
    let client = xray::client::XrayApiClient::new(backend.as_ref());

    client.rename_user(old_name, new_name).await?;

    println!("User renamed: '{}' -> '{}'", old_name, new_name);
    println!("Note: rename resets traffic stats history for this user.");

    Ok(())
}

async fn cli_deploy_bot(config: &Config, token: &str) -> error::Result<()> {
    // TODO(Epic D Task 2.x): reinstate token format validation after helper relocation.
    if token.trim().is_empty() {
        return Err(error::AppError::Config(
            "Invalid token format (expected <digits>:<secret>)".to_string(),
        ));
    }

    eprintln!("Connecting to VPS...");
    eprintln!("Deploying Telegram bot...");

    match deploy_bot(config, token).await {
        Ok(msg) => {
            println!("{}", msg);
            Ok(())
        }
        Err(e) => Err(error::AppError::Xray(e)),
    }
}

#[cfg(test)]
mod tests {
    // Tests ported from the deleted src/backend.rs for the helpers now inlined above.
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
            bot_image: Default::default(),
            snapshot_dir: None,
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
            bot_image: Default::default(),
            snapshot_dir: None,
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
            bot_image: Default::default(),
            snapshot_dir: None,
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
            bot_image: Default::default(),
            snapshot_dir: None,
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
        // Rate-limit or error bodies, prerelease tags, etc. must not render as versions.
        let body = r#"{"tag_name":"rate limit exceeded"}"#;
        assert_eq!(parse_xray_version_from_json(body), None);
        let body = r#"{"tag_name":"v1.0-beta"}"#;
        assert_eq!(parse_xray_version_from_json(body), None);
        let body = r#"{"tag_name":"v"}"#;
        assert_eq!(parse_xray_version_from_json(body), None);
        let body = r#"{"tag_name":""}"#;
        assert_eq!(parse_xray_version_from_json(body), None);
    }

    #[tokio::test]
    async fn test_deploy_bot_requires_admin_id() {
        let config = Config {
            host: Some("1.2.3.4".to_string()),
            port: 22,
            user: "root".to_string(),
            key_path: None,
            ssh_host: None,
            container: "amnezia-xray".to_string(),
            telegram_token: None,
            telegram_admin_chat_id: None,
            bot_image: Default::default(), // no admin_id
            snapshot_dir: None,
        };
        let result = deploy_bot(&config, "123:abc").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("admin_id is required"));
    }

    #[tokio::test]
    async fn test_deploy_bot_with_admin_id_attempts_connection() {
        let config = Config {
            host: Some("192.0.2.1".to_string()), // non-routable IP
            port: 22,
            user: "root".to_string(),
            key_path: None,
            ssh_host: None,
            container: "amnezia-xray".to_string(),
            telegram_token: None,
            telegram_admin_chat_id: Some(123456789),
            bot_image: Default::default(),
            snapshot_dir: None,
        };
        // With admin_id set, it should pass the admin_id check and fail at SSH connection
        let result = deploy_bot(&config, "123:abc").await;
        assert!(result.is_err());
        // Should NOT be the "admin_id is required" error
        let err = result.unwrap_err();
        assert!(!err.contains("admin_id is required"), "got: {}", err);
    }
}
