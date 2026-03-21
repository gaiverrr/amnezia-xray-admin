mod app;
mod backend;
mod backend_trait;
mod config;
mod error;
mod ssh;
mod telegram;
mod ui;
mod xray;

use clap::Parser;
use config::{Cli, Config};
use backend_trait::{LocalBackend, XrayBackend};

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

    if cli.check_server {
        if let Err(e) = runtime.block_on(cli_check_server(&config, local)) {
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
        let token = match cli.telegram_token {
            Some(ref t) => t.clone(),
            None => {
                eprintln!("Error: --telegram-token or TELEGRAM_TOKEN env var is required");
                std::process::exit(1);
            }
        };
        if let Err(e) = runtime.block_on(cli_telegram_bot(&config, &token, local)) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    // Initialize terminal
    let mut terminal = match app::init_terminal() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Failed to initialize terminal: {}", e);
            std::process::exit(1);
        }
    };

    // Create app and run event loop
    let mut application = app::App::with_config(config, runtime.handle().clone());
    let result = app::run(&mut application, &mut terminal);

    // Always restore terminal, even on error
    if let Err(e) = app::restore_terminal(&mut terminal) {
        eprintln!("Failed to restore terminal: {}", e);
    }

    if let Err(e) = result {
        eprintln!("Application error: {}", e);
        std::process::exit(1);
    }
}

/// Create a backend for CLI commands: either LocalBackend (--local) or SshBackend (default).
async fn connect_cli_backend(config: &Config, local: bool) -> error::Result<Box<dyn XrayBackend>> {
    if local {
        let hostname = get_local_hostname().await;
        Ok(Box::new(LocalBackend::new(
            config.container.clone(),
            hostname,
        )))
    } else {
        let backend = backend::connect_backend(config).await?;
        Ok(Box::new(backend))
    }
}

/// Get the local machine's hostname for vless URL generation.
async fn get_local_hostname() -> String {
    // Try to get the primary IP address
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
    // Fallback to hostname
    if let Ok(output) = tokio::process::Command::new("hostname").output().await {
        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !name.is_empty() {
            return name;
        }
    }
    "localhost".to_string()
}

async fn cli_check_server(config: &Config, local: bool) -> error::Result<()> {
    if !local {
        let (_, port, user, _) = backend::resolve_connection_info(config)?;
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

    // Ensure API is enabled (idempotent — no restart if already configured)
    eprintln!("Checking API configuration...");
    let modified = xray::config::ensure_api_enabled(backend.as_ref()).await?;
    if modified {
        eprintln!("API was not configured — enabled and container restarted.");
    } else {
        eprintln!("API already configured.");
    }

    let client = xray::client::XrayApiClient::new(backend.as_ref());

    let users = client.list_users().await?;
    let server_info = client.get_server_info().await?;

    println!(
        "API enabled, {} users, xray v{}",
        users.len(),
        server_info.version
    );

    Ok(())
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

    let vless_url = backend::build_vless_url(backend.as_ref(), &user.uuid, &user.name).await?;

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

    let vless_url = backend::build_vless_url(backend.as_ref(), &user.uuid, &user.name).await?;

    match ui::qr::render_qr_to_lines(&vless_url) {
        Ok(lines) => {
            for line in &lines {
                println!("{}", line);
            }
            println!();
            println!("{}", name);
            println!("{}", vless_url);
        }
        Err(e) => {
            return Err(error::AppError::Xray(format!(
                "QR generation failed: {}",
                e
            )));
        }
    }

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
    let api_status = if server_info.version != "unknown" {
        "enabled"
    } else {
        "unknown"
    };

    println!("Xray version:  v{}", server_info.version);
    println!("API status:    {}", api_status);
    println!("Users:         {}", users.len());
    println!(
        "Total upload:  {}",
        ui::dashboard::format_bytes(server_info.uplink)
    );
    println!(
        "Total download: {}",
        ui::dashboard::format_bytes(server_info.downlink)
    );

    Ok(())
}

async fn cli_list_users(config: &Config, local: bool) -> error::Result<()> {
    if !local {
        let (_, port, user, _) = backend::resolve_connection_info(config)?;
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
            &user.uuid[..8]
        } else {
            &user.name
        };
        let uuid_short = &user.uuid[..std::cmp::min(8, user.uuid.len())];
        let online = if user.online_count > 0 {
            format!("● {}", user.online_count)
        } else {
            "○".to_string()
        };

        println!(
            "{:<30} {:<10} {:<12} {:<12} {:<8}",
            name,
            uuid_short,
            ui::dashboard::format_bytes(user.stats.uplink),
            ui::dashboard::format_bytes(user.stats.downlink),
            online,
        );
    }

    Ok(())
}

async fn cli_telegram_bot(config: &Config, token: &str, local: bool) -> error::Result<()> {
    env_logger::init();
    let backend = connect_cli_backend(config, local).await?;
    telegram::run_bot(token, backend, config.clone()).await
}
