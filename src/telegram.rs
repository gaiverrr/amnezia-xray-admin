//! Telegram bot for managing Xray users via Telegram commands.
//!
//! Auto-admin: the first user to send `/start` becomes the admin.
//! Their chat ID is persisted to config so subsequent starts skip setup.

use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::ChatId;
use teloxide::utils::command::BotCommands;
use tokio::sync::Mutex;

use crate::backend_trait::XrayBackend;
use crate::config::Config;
use crate::error::Result;
use crate::ui::dashboard::format_bytes;
use crate::xray::client::{ServerInfo, XrayApiClient};
use crate::xray::types::{TrafficStats, XrayUser};

/// State shared across all Telegram handlers.
pub struct BotState {
    pub backend: Box<dyn XrayBackend>,
    pub config: Mutex<Config>,
}

/// Commands recognized by the bot.
#[derive(BotCommands, Clone, Debug)]
#[command(rename_rule = "lowercase")]
pub enum Command {
    /// Register as admin (first user only) and show welcome
    Start,
    /// Show available commands
    Help,
    /// List users with traffic stats
    Users,
    /// Server info and online users
    Status,
}

/// Check if a chat ID is the admin. Returns true if no admin is set yet (first-time setup).
fn is_admin_or_unset(config: &Config, chat_id: ChatId) -> bool {
    match config.telegram_admin_chat_id {
        None => true,
        Some(admin_id) => chat_id.0 == admin_id,
    }
}

/// Format the /help response text.
pub fn help_text() -> String {
    [
        "Available commands:",
        "/start - Register as admin / show welcome",
        "/help - Show this help message",
        "/users - List users with stats",
        "/add <name> - Add a new user",
        "/delete <name> - Delete a user",
        "/url <name> - Get vless:// URL",
        "/qr <name> - Get QR code image",
        "/status - Server info + online users",
    ]
    .join("\n")
}

/// Format the welcome message shown after /start.
pub fn welcome_text(is_new_admin: bool) -> String {
    if is_new_admin {
        [
            "Welcome! You are now the admin of this bot.",
            "",
            "Use /help to see available commands.",
        ]
        .join("\n")
    } else {
        [
            "Welcome back!",
            "",
            "Use /help to see available commands.",
        ]
        .join("\n")
    }
}

/// Format the /users response: list users with traffic stats.
pub fn format_users_message(users: &[(XrayUser, TrafficStats, u32)]) -> String {
    if users.is_empty() {
        return "No users found.".to_string();
    }

    let mut lines = Vec::new();
    lines.push("👥 Users:".to_string());
    lines.push(String::new());

    for (user, stats, online_count) in users {
        let name = if user.name.is_empty() {
            &user.uuid[..std::cmp::min(8, user.uuid.len())]
        } else {
            &user.name
        };
        let online_indicator = if *online_count > 0 {
            format!("🟢 {}", online_count)
        } else {
            "⚪".to_string()
        };
        lines.push(format!(
            "{} {} ↑{} ↓{}",
            online_indicator,
            name,
            format_bytes(stats.uplink),
            format_bytes(stats.downlink),
        ));
    }

    lines.join("\n")
}

/// Format the /status response: server info and online users summary.
pub fn format_status_message(
    server_info: &ServerInfo,
    user_count: usize,
    online_count: usize,
) -> String {
    let mut lines = Vec::new();
    lines.push("📊 Server Status:".to_string());
    lines.push(String::new());
    lines.push(format!("Xray version: v{}", server_info.version));
    lines.push(format!("Users: {} ({} online)", user_count, online_count));
    lines.push(format!("Upload: {}", format_bytes(server_info.uplink)));
    lines.push(format!("Download: {}", format_bytes(server_info.downlink)));

    lines.join("\n")
}

/// Handle incoming bot commands.
async fn handle_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
    state: Arc<BotState>,
) -> ResponseResult<()> {
    let chat_id = msg.chat.id;

    // Check admin access (hold lock briefly)
    {
        let mut config = state.config.lock().await;

        if !is_admin_or_unset(&config, chat_id) {
            bot.send_message(chat_id, "Access denied. Only the admin can use this bot.")
                .await?;
            return Ok(());
        }

        // Handle /start which needs to write config
        if let Command::Start = &cmd {
            let is_new = config.telegram_admin_chat_id.is_none();
            if is_new {
                config.telegram_admin_chat_id = Some(chat_id.0);
                if let Err(e) = config.save() {
                    log::error!("Failed to save admin chat ID: {}", e);
                }
                log::info!("Admin registered: chat_id={}", chat_id.0);
            }
            bot.send_message(chat_id, welcome_text(is_new)).await?;
            return Ok(());
        }
    }
    // Config lock is dropped here — safe to do async backend work

    match cmd {
        Command::Start => unreachable!(), // handled above
        Command::Help => {
            bot.send_message(chat_id, help_text()).await?;
        }
        Command::Users => {
            let text = match cmd_users(&state).await {
                Ok(t) => t,
                Err(e) => format!("Error: {}", e),
            };
            bot.send_message(chat_id, text).await?;
        }
        Command::Status => {
            let text = match cmd_status(&state).await {
                Ok(t) => t,
                Err(e) => format!("Error: {}", e),
            };
            bot.send_message(chat_id, text).await?;
        }
    }

    Ok(())
}

/// Execute /users command: list users with stats.
async fn cmd_users(state: &BotState) -> std::result::Result<String, crate::error::AppError> {
    let client = XrayApiClient::new(state.backend.as_ref());
    let users = client.list_users().await?;

    let mut user_data = Vec::new();
    for user in users {
        let stats = client.get_user_stats(&user.email).await.unwrap_or_default();
        let online = client.get_online_count(&user.email).await.unwrap_or(0);
        user_data.push((user, stats, online));
    }

    Ok(format_users_message(&user_data))
}

/// Execute /status command: server info + online summary.
async fn cmd_status(state: &BotState) -> std::result::Result<String, crate::error::AppError> {
    let client = XrayApiClient::new(state.backend.as_ref());
    let server_info = client.get_server_info().await?;
    let users = client.list_users().await?;

    let mut online_total = 0usize;
    for user in &users {
        let count = client.get_online_count(&user.email).await.unwrap_or(0);
        online_total += count as usize;
    }

    Ok(format_status_message(&server_info, users.len(), online_total))
}

/// Start the Telegram bot and block until shutdown.
pub async fn run_bot(token: &str, backend: Box<dyn XrayBackend>, config: Config) -> Result<()> {
    log::info!("Starting Telegram bot...");

    let bot = Bot::new(token);

    let state = Arc::new(BotState {
        backend,
        config: Mutex::new(config),
    });

    let handler = Update::filter_message().filter_command::<Command>().endpoint(
        move |bot: Bot, msg: Message, cmd: Command| {
            let state = Arc::clone(&state);
            async move { handle_command(bot, msg, cmd, state).await }
        },
    );

    Dispatcher::builder(bot, handler)
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_help_text_contains_all_commands() {
        let text = help_text();
        assert!(text.contains("/start"));
        assert!(text.contains("/help"));
        assert!(text.contains("/users"));
        assert!(text.contains("/add"));
        assert!(text.contains("/delete"));
        assert!(text.contains("/url"));
        assert!(text.contains("/qr"));
        assert!(text.contains("/status"));
    }

    #[test]
    fn test_welcome_text_new_admin() {
        let text = welcome_text(true);
        assert!(text.contains("You are now the admin"));
        assert!(text.contains("/help"));
    }

    #[test]
    fn test_welcome_text_returning_admin() {
        let text = welcome_text(false);
        assert!(text.contains("Welcome back"));
        assert!(text.contains("/help"));
    }

    #[test]
    fn test_is_admin_or_unset_no_admin() {
        let config = Config::default();
        assert!(is_admin_or_unset(&config, ChatId(12345)));
    }

    #[test]
    fn test_is_admin_or_unset_matching_admin() {
        let mut config = Config::default();
        config.telegram_admin_chat_id = Some(12345);
        assert!(is_admin_or_unset(&config, ChatId(12345)));
    }

    #[test]
    fn test_is_admin_or_unset_wrong_user() {
        let mut config = Config::default();
        config.telegram_admin_chat_id = Some(12345);
        assert!(!is_admin_or_unset(&config, ChatId(99999)));
    }

    #[test]
    fn test_bot_commands_parse() {
        let cmds = Command::bot_commands();
        let descriptions: String = cmds.iter().map(|c| c.command.as_str()).collect::<Vec<_>>().join(",");
        // teloxide BotCommand.command contains just the name (no slash)
        assert!(descriptions.contains("start"), "commands: {}", descriptions);
        assert!(descriptions.contains("help"), "commands: {}", descriptions);
        assert!(descriptions.contains("users"), "commands: {}", descriptions);
        assert!(descriptions.contains("status"), "commands: {}", descriptions);
    }

    #[test]
    fn test_help_text_not_empty() {
        let text = help_text();
        assert!(!text.is_empty());
        assert!(text.lines().count() >= 5);
    }

    #[test]
    fn test_format_users_message_empty() {
        let text = format_users_message(&[]);
        assert_eq!(text, "No users found.");
    }

    #[test]
    fn test_format_users_message_with_users() {
        let users = vec![
            (
                XrayUser {
                    uuid: "aaaa-bbbb-cccc".to_string(),
                    name: "Alice".to_string(),
                    email: "Alice@vpn".to_string(),
                    flow: "xtls-rprx-vision".to_string(),
                    stats: TrafficStats::default(),
                    online_count: 0,
                },
                TrafficStats { uplink: 1024 * 1024 * 100, downlink: 1024 * 1024 * 1024 * 2 },
                1,
            ),
            (
                XrayUser {
                    uuid: "dddd-eeee-ffff".to_string(),
                    name: "Bob".to_string(),
                    email: "Bob@vpn".to_string(),
                    flow: "xtls-rprx-vision".to_string(),
                    stats: TrafficStats::default(),
                    online_count: 0,
                },
                TrafficStats { uplink: 0, downlink: 0 },
                0,
            ),
        ];
        let text = format_users_message(&users);
        assert!(text.contains("Alice"), "text: {}", text);
        assert!(text.contains("Bob"), "text: {}", text);
        // Alice is online (count=1)
        assert!(text.contains("🟢 1"), "text: {}", text);
        // Bob is offline (count=0)
        assert!(text.contains("⚪"), "text: {}", text);
        // Traffic stats present
        assert!(text.contains("100.0 MB"), "text: {}", text);
        assert!(text.contains("2.0 GB"), "text: {}", text);
    }

    #[test]
    fn test_format_users_message_unnamed_user() {
        let users = vec![(
            XrayUser {
                uuid: "12345678-abcd-efgh".to_string(),
                name: String::new(),
                email: "@vpn".to_string(),
                flow: String::new(),
                stats: TrafficStats::default(),
                online_count: 0,
            },
            TrafficStats::default(),
            0,
        )];
        let text = format_users_message(&users);
        // Should show truncated UUID for unnamed users
        assert!(text.contains("12345678"), "text: {}", text);
    }

    #[test]
    fn test_format_status_message() {
        let info = ServerInfo {
            version: "1.8.4".to_string(),
            uplink: 1024 * 1024 * 500,
            downlink: 1024 * 1024 * 1024 * 10,
        };
        let text = format_status_message(&info, 5, 2);
        assert!(text.contains("v1.8.4"), "text: {}", text);
        assert!(text.contains("5"), "text: {}", text);
        assert!(text.contains("2 online"), "text: {}", text);
        assert!(text.contains("500.0 MB"), "text: {}", text);
        assert!(text.contains("10.0 GB"), "text: {}", text);
    }

    #[test]
    fn test_format_status_message_zero_online() {
        let info = ServerInfo {
            version: "1.8.0".to_string(),
            uplink: 0,
            downlink: 0,
        };
        let text = format_status_message(&info, 3, 0);
        assert!(text.contains("0 online"), "text: {}", text);
        assert!(text.contains("3"), "text: {}", text);
    }

    #[test]
    fn test_config_telegram_admin_serialization() {
        let mut config = Config::default();
        config.telegram_admin_chat_id = Some(123456789);
        let toml_str = toml::to_string_pretty(&config).unwrap();
        assert!(toml_str.contains("telegram_admin_chat_id = 123456789"));

        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.telegram_admin_chat_id, Some(123456789));
    }

    #[test]
    fn test_config_telegram_admin_none_not_serialized() {
        let config = Config::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        assert!(!toml_str.contains("telegram_admin_chat_id"));
    }
}
