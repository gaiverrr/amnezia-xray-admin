mod app;
mod backend;
mod config;
mod error;
mod ssh;
mod ui;
mod xray;

use clap::Parser;
use config::{Cli, Config};

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
        if let Err(e) = runtime.block_on(cli_list_users(&config)) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    if cli.check_server {
        if let Err(e) = runtime.block_on(cli_check_server(&config)) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    if let Some(ref name) = cli.user_url {
        if let Err(e) = runtime.block_on(cli_user_url(&config, name)) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    if let Some(ref name) = cli.user_qr {
        if let Err(e) = runtime.block_on(cli_user_qr(&config, name)) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    if cli.online_status {
        if let Err(e) = runtime.block_on(cli_online_status(&config)) {
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

async fn cli_check_server(config: &Config) -> error::Result<()> {
    let (hostname, port, user, key_path) = backend::resolve_connection_info(config)?;
    let addr = if hostname.contains(':') {
        format!("[{}]:{}", hostname, port)
    } else {
        format!("{}:{}", hostname, port)
    };

    eprintln!("Connecting to {}@{}...", user, addr);
    let session =
        ssh::SshSession::connect(&addr, &user, key_path.as_deref(), &config.container).await?;

    // Ensure API is enabled (idempotent — no restart if already configured)
    eprintln!("Checking API configuration...");
    let modified = xray::config::ensure_api_enabled(&session, &config.container).await?;
    if modified {
        eprintln!("API was not configured — enabled and container restarted.");
    } else {
        eprintln!("API already configured.");
    }

    let client = xray::client::XrayApiClient::new(&session);

    let users = client.list_users().await?;
    let server_info = client.get_server_info().await?;

    println!(
        "API enabled, {} users, xray v{}",
        users.len(),
        server_info.version
    );

    let _ = session.close().await;
    Ok(())
}

async fn cli_user_url(config: &Config, name: &str) -> error::Result<()> {
    let session = backend::connect(config).await?;
    let client = xray::client::XrayApiClient::new(&session);
    let users = client.list_users().await?;

    let user = users.iter().find(|u| u.name == name);
    let user = match user {
        Some(u) => u,
        None => {
            let _ = session.close().await;
            return Err(error::AppError::Xray(format!(
                "user '{}' not found",
                name
            )));
        }
    };

    let vless_url = backend::build_vless_url(&session, config, &user.uuid, &user.name).await?;
    let _ = session.close().await;

    println!("{}", vless_url);
    Ok(())
}

async fn cli_user_qr(config: &Config, name: &str) -> error::Result<()> {
    let session = backend::connect(config).await?;
    let client = xray::client::XrayApiClient::new(&session);
    let users = client.list_users().await?;

    let user = users.iter().find(|u| u.name == name);
    let user = match user {
        Some(u) => u,
        None => {
            let _ = session.close().await;
            return Err(error::AppError::Xray(format!(
                "user '{}' not found",
                name
            )));
        }
    };

    let vless_url = backend::build_vless_url(&session, config, &user.uuid, &user.name).await?;
    let _ = session.close().await;

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
            return Err(error::AppError::Xray(format!("QR generation failed: {}", e)));
        }
    }

    Ok(())
}

async fn cli_online_status(config: &Config) -> error::Result<()> {
    let session = backend::connect(config).await?;
    let client = xray::client::XrayApiClient::new(&session);
    let users = client.list_users().await?;

    if users.is_empty() {
        println!("No users found.");
        let _ = session.close().await;
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

    let _ = session.close().await;

    // Print table
    println!(
        "{:<30} {:<8} IPs",
        "NAME", "ONLINE"
    );
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

async fn cli_list_users(config: &Config) -> error::Result<()> {
    let (hostname, port, user, key_path) = backend::resolve_connection_info(config)?;
    let addr = if hostname.contains(':') {
        format!("[{}]:{}", hostname, port)
    } else {
        format!("{}:{}", hostname, port)
    };

    eprintln!("Connecting to {}@{}...", user, addr);
    let session = ssh::SshSession::connect(&addr, &user, key_path.as_deref(), &config.container).await?;

    let client = xray::client::XrayApiClient::new(&session);
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
