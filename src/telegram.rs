//! Telegram bot for managing Xray users via Telegram commands.
//!
//! Admin is set at deploy time via `--admin-id`. Only the configured admin
//! can interact with the bot; all other users get "Access denied".

use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{ChatId, InlineKeyboardButton, InlineKeyboardMarkup, InputFile};
use teloxide::utils::command::BotCommands;
use tokio::sync::Mutex;

use crate::backend;
use crate::backend_trait::XrayBackend;
use crate::config::Config;
use crate::error::Result;
use crate::ui::dashboard::format_bytes;
use crate::ui::qr::render_qr_to_png;
use crate::xray::client::{ServerInfo, XrayApiClient};
use crate::xray::types::{TrafficStats, XrayUser};

/// State shared across all Telegram handlers.
pub struct BotState {
    pub backend: Box<dyn XrayBackend>,
    pub config: Mutex<Config>,
}

/// Callback query prefix for URL inline buttons.
const URL_PREFIX: &str = "url:";
/// Callback query prefix for delete confirmation buttons.
const DELETE_CONFIRM_PREFIX: &str = "delete_confirm:";
/// Callback query prefix for delete cancel buttons.
const DELETE_CANCEL_PREFIX: &str = "delete_cancel:";

/// Commands recognized by the bot.
#[derive(BotCommands, Clone, Debug)]
#[command(rename_rule = "lowercase")]
pub enum Command {
    /// Show welcome message (admin only)
    Start,
    /// Show available commands
    Help,
    /// List users with traffic stats
    Users,
    /// Server info and online users
    Status,
    /// Add a new user
    #[command(description = "Add a new user")]
    Add(String),
    /// Delete a user
    #[command(description = "Delete a user")]
    Delete(String),
    /// Get vless:// URL for a user
    #[command(description = "Get vless:// URL")]
    Url(String),
    /// Get QR code image for a user
    #[command(description = "Get QR code image")]
    Qr(String),
}

/// Check if a chat ID matches the configured admin.
fn is_admin(config: &Config, chat_id: ChatId) -> bool {
    config.telegram_admin_chat_id == Some(chat_id.0)
}

/// Format the /help response text.
pub fn help_text() -> String {
    [
        "Available commands:",
        "/start - Show welcome message",
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

/// Format the welcome message shown after /start for the admin.
pub fn welcome_text() -> String {
    ["Welcome, admin!", "", "Use /help to see available commands."].join("\n")
}

/// Format the access denied message for non-admin users.
pub fn access_denied_text() -> String {
    "Access denied. Contact the server administrator.".to_string()
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

/// Format the /add success response.
pub fn format_add_message(name: &str, uuid: &str, vless_url: &str) -> String {
    [
        format!("✅ User '{}' added.", name),
        String::new(),
        format!("UUID: {}", uuid),
        format!("URL: {}", vless_url),
    ]
    .join("\n")
}

/// Format the /delete confirmation prompt.
pub fn format_delete_confirm_message(name: &str) -> String {
    format!(
        "⚠️ Delete user '{}'?\n\nThis will revoke their access immediately.",
        name
    )
}

/// Format the /delete success response.
pub fn format_delete_success_message(name: &str) -> String {
    format!("🗑 User '{}' deleted.", name)
}

/// Validate a user name for /add command.
/// Returns an error message if invalid, None if valid.
pub fn validate_user_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Some("Usage: /add <name>".to_string());
    }
    if trimmed.len() > 64 {
        return Some("Name too long (max 64 characters).".to_string());
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Some("Name must not contain control characters.".to_string());
    }
    if trimmed.contains('@') || trimmed.contains(">>>") {
        return Some("Name must not contain '@' or '>>>'.".to_string());
    }
    None
}

/// Build an inline keyboard listing users, with each button using the given callback prefix.
///
/// Each button shows the user's name and has callback data `"{prefix}{name}"`.
/// Users with empty names are skipped.
pub fn build_user_keyboard(users: &[XrayUser], callback_prefix: &str) -> InlineKeyboardMarkup {
    let buttons: Vec<Vec<InlineKeyboardButton>> = users
        .iter()
        .filter(|u| !u.name.is_empty())
        .map(|u| {
            vec![InlineKeyboardButton::callback(
                &u.name,
                format!("{}{}", callback_prefix, u.name),
            )]
        })
        .collect();
    InlineKeyboardMarkup::new(buttons)
}

/// Build an inline keyboard for delete confirmation.
pub fn delete_confirmation_keyboard(user_uuid: &str) -> InlineKeyboardMarkup {
    let confirm = InlineKeyboardButton::callback(
        "Yes, delete",
        format!("{}{}", DELETE_CONFIRM_PREFIX, user_uuid),
    );
    let cancel =
        InlineKeyboardButton::callback("Cancel", format!("{}{}", DELETE_CANCEL_PREFIX, user_uuid));
    InlineKeyboardMarkup::new(vec![vec![confirm, cancel]])
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

    // Check admin access
    {
        let config = state.config.lock().await;

        if let Command::Start = &cmd {
            if is_admin(&config, chat_id) {
                bot.send_message(chat_id, welcome_text()).await?;
            } else {
                bot.send_message(chat_id, access_denied_text()).await?;
            }
            return Ok(());
        }

        if !is_admin(&config, chat_id) {
            bot.send_message(chat_id, access_denied_text()).await?;
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
        Command::Add(name) => {
            let name = name.trim().to_string();
            if let Some(err) = validate_user_name(&name) {
                bot.send_message(chat_id, err).await?;
            } else {
                let text = match cmd_add(&state, &name).await {
                    Ok(t) => t,
                    Err(e) => format!("Error: {}", e),
                };
                bot.send_message(chat_id, text).await?;
            }
        }
        Command::Delete(name) => {
            let name = name.trim().to_string();
            if name.is_empty() {
                bot.send_message(chat_id, "Usage: /delete <name>").await?;
            } else {
                match cmd_delete_prompt(&state, &name).await {
                    Ok((text, keyboard)) => {
                        bot.send_message(chat_id, text)
                            .reply_markup(keyboard)
                            .await?;
                    }
                    Err(e) => {
                        bot.send_message(chat_id, format!("Error: {}", e)).await?;
                    }
                }
            }
        }
        Command::Url(name) => {
            let name = name.trim().to_string();
            if name.is_empty() {
                match cmd_user_keyboard(&state, URL_PREFIX).await {
                    Ok(keyboard) => {
                        bot.send_message(chat_id, "Select a user:")
                            .reply_markup(keyboard)
                            .await?;
                    }
                    Err(e) => {
                        bot.send_message(chat_id, format!("Error: {}", e)).await?;
                    }
                }
            } else {
                let text = match cmd_url(&state, &name).await {
                    Ok(t) => t,
                    Err(e) => format!("Error: {}", e),
                };
                bot.send_message(chat_id, text).await?;
            }
        }
        Command::Qr(name) => {
            let name = name.trim().to_string();
            if name.is_empty() {
                bot.send_message(chat_id, "Usage: /qr <name>").await?;
            } else {
                match cmd_qr(&state, &name).await {
                    Ok((png_bytes, caption)) => {
                        let input = InputFile::memory(png_bytes).file_name("qr.png");
                        bot.send_photo(chat_id, input).caption(caption).await?;
                    }
                    Err(e) => {
                        bot.send_message(chat_id, format!("Error: {}", e)).await?;
                    }
                }
            }
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

/// Execute /add command: add a user, return UUID + vless URL.
async fn cmd_add(
    state: &BotState,
    name: &str,
) -> std::result::Result<String, crate::error::AppError> {
    let client = XrayApiClient::new(state.backend.as_ref());
    let uuid = client.add_user(name).await?;
    let vless_url = backend::build_vless_url(state.backend.as_ref(), &uuid, name).await?;
    Ok(format_add_message(name, &uuid, &vless_url))
}

/// Execute /delete prompt: find user and return confirmation message with inline keyboard.
async fn cmd_delete_prompt(
    state: &BotState,
    name: &str,
) -> std::result::Result<(String, InlineKeyboardMarkup), crate::error::AppError> {
    let client = XrayApiClient::new(state.backend.as_ref());
    let users = client.list_users().await?;

    let user = users
        .iter()
        .find(|u| u.name == name)
        .ok_or_else(|| crate::error::AppError::Xray(format!("user '{}' not found", name)))?;

    let text = format_delete_confirm_message(name);
    let keyboard = delete_confirmation_keyboard(&user.uuid);
    Ok((text, keyboard))
}

/// Execute the actual user deletion by UUID.
async fn cmd_delete_execute(
    state: &BotState,
    uuid: &str,
) -> std::result::Result<String, crate::error::AppError> {
    let client = XrayApiClient::new(state.backend.as_ref());

    // Look up user name before deletion for the response message
    let users = client.list_users().await?;
    let name = users
        .iter()
        .find(|u| u.uuid == uuid)
        .map(|u| u.name.clone())
        .unwrap_or_else(|| uuid[..std::cmp::min(8, uuid.len())].to_string());

    client.remove_user(uuid).await?;
    Ok(format_delete_success_message(&name))
}

/// Build an inline keyboard listing all users, for use by /url, /qr, /delete without arguments.
async fn cmd_user_keyboard(
    state: &BotState,
    callback_prefix: &str,
) -> std::result::Result<InlineKeyboardMarkup, crate::error::AppError> {
    let client = XrayApiClient::new(state.backend.as_ref());
    let users = client.list_users().await?;
    Ok(build_user_keyboard(&users, callback_prefix))
}

/// Execute /url command: get vless:// URL for a user.
async fn cmd_url(
    state: &BotState,
    name: &str,
) -> std::result::Result<String, crate::error::AppError> {
    let client = XrayApiClient::new(state.backend.as_ref());
    let users = client.list_users().await?;

    let user = users
        .iter()
        .find(|u| u.name == name)
        .ok_or_else(|| crate::error::AppError::Xray(format!("user '{}' not found", name)))?;

    let vless_url = backend::build_vless_url(state.backend.as_ref(), &user.uuid, name).await?;
    Ok(format_url_message(name, &vless_url))
}

/// Execute /qr command: generate QR code PNG for a user's vless URL.
async fn cmd_qr(
    state: &BotState,
    name: &str,
) -> std::result::Result<(Vec<u8>, String), crate::error::AppError> {
    let client = XrayApiClient::new(state.backend.as_ref());
    let users = client.list_users().await?;

    let user = users
        .iter()
        .find(|u| u.name == name)
        .ok_or_else(|| crate::error::AppError::Xray(format!("user '{}' not found", name)))?;

    let vless_url = backend::build_vless_url(state.backend.as_ref(), &user.uuid, name).await?;

    let png_bytes = render_qr_to_png(&vless_url, 8)
        .map_err(|e| crate::error::AppError::Xray(format!("QR generation failed: {}", e)))?;

    let caption = format!("🔗 {}\n\n{}", name, vless_url);
    Ok((png_bytes, caption))
}

/// Format the /url response: vless:// URL as a copyable message.
pub fn format_url_message(name: &str, vless_url: &str) -> String {
    format!("🔗 {} URL:\n\n{}", name, vless_url)
}

/// Handle callback queries from inline keyboard buttons (e.g., delete confirmation).
async fn handle_callback(bot: Bot, q: CallbackQuery, state: Arc<BotState>) -> ResponseResult<()> {
    let data = match q.data {
        Some(ref d) => d.as_str(),
        None => return Ok(()),
    };

    let chat_id = match q.message {
        Some(ref msg) => msg.chat().id,
        None => return Ok(()),
    };

    // Check admin access
    {
        let config = state.config.lock().await;
        if !is_admin(&config, chat_id) {
            bot.answer_callback_query(q.id.clone())
                .text("Access denied.")
                .await?;
            return Ok(());
        }
    }

    if let Some(user_name) = data.strip_prefix(URL_PREFIX) {
        let text = match cmd_url(&state, user_name).await {
            Ok(t) => t,
            Err(e) => format!("Error: {}", e),
        };
        bot.answer_callback_query(q.id.clone()).await?;
        if let Some(ref msg) = q.message {
            bot.edit_message_text(chat_id, msg.id(), &text).await?;
        }
    } else if let Some(uuid) = data.strip_prefix(DELETE_CONFIRM_PREFIX) {
        let text = match cmd_delete_execute(&state, uuid).await {
            Ok(t) => t,
            Err(e) => format!("Error: {}", e),
        };
        bot.answer_callback_query(q.id.clone()).await?;
        // Edit the original message to show the result
        if let Some(ref msg) = q.message {
            bot.edit_message_text(chat_id, msg.id(), &text).await?;
        }
    } else if data.starts_with(DELETE_CANCEL_PREFIX) {
        bot.answer_callback_query(q.id.clone()).await?;
        if let Some(ref msg) = q.message {
            bot.edit_message_text(chat_id, msg.id(), "Deletion cancelled.")
                .await?;
        }
    }

    Ok(())
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

    Ok(format_status_message(
        &server_info,
        users.len(),
        online_total,
    ))
}

/// Start the Telegram bot and block until shutdown.
pub async fn run_bot(token: &str, backend: Box<dyn XrayBackend>, config: Config) -> Result<()> {
    log::info!("Starting Telegram bot...");

    let bot = Bot::new(token);

    let state = Arc::new(BotState {
        backend,
        config: Mutex::new(config),
    });

    let state_cmd = Arc::clone(&state);
    let state_cb = Arc::clone(&state);

    let handler = dptree::entry()
        .branch(
            Update::filter_message()
                .filter_command::<Command>()
                .endpoint(move |bot: Bot, msg: Message, cmd: Command| {
                    let state = Arc::clone(&state_cmd);
                    async move { handle_command(bot, msg, cmd, state).await }
                }),
        )
        .branch(
            Update::filter_callback_query().endpoint(move |bot: Bot, q: CallbackQuery| {
                let state = Arc::clone(&state_cb);
                async move { handle_callback(bot, q, state).await }
            }),
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
    fn test_welcome_text() {
        let text = welcome_text();
        assert!(text.contains("Welcome, admin"));
        assert!(text.contains("/help"));
    }

    #[test]
    fn test_access_denied_text() {
        let text = access_denied_text();
        assert!(text.contains("Access denied"));
        assert!(text.contains("administrator"));
    }

    #[test]
    fn test_is_admin_no_admin_configured() {
        let config = Config::default();
        assert!(!is_admin(&config, ChatId(12345)));
    }

    #[test]
    fn test_is_admin_matching() {
        let mut config = Config::default();
        config.telegram_admin_chat_id = Some(12345);
        assert!(is_admin(&config, ChatId(12345)));
    }

    #[test]
    fn test_is_admin_wrong_user() {
        let mut config = Config::default();
        config.telegram_admin_chat_id = Some(12345);
        assert!(!is_admin(&config, ChatId(99999)));
    }

    #[test]
    fn test_bot_commands_parse() {
        let cmds = Command::bot_commands();
        let descriptions: String = cmds
            .iter()
            .map(|c| c.command.as_str())
            .collect::<Vec<_>>()
            .join(",");
        // teloxide BotCommand.command contains just the name (no slash)
        assert!(descriptions.contains("start"), "commands: {}", descriptions);
        assert!(descriptions.contains("help"), "commands: {}", descriptions);
        assert!(descriptions.contains("users"), "commands: {}", descriptions);
        assert!(
            descriptions.contains("status"),
            "commands: {}",
            descriptions
        );
        assert!(descriptions.contains("add"), "commands: {}", descriptions);
        assert!(
            descriptions.contains("delete"),
            "commands: {}",
            descriptions
        );
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
                TrafficStats {
                    uplink: 1024 * 1024 * 100,
                    downlink: 1024 * 1024 * 1024 * 2,
                },
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
                TrafficStats {
                    uplink: 0,
                    downlink: 0,
                },
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

    // -- /add and /delete formatting tests --

    #[test]
    fn test_format_add_message() {
        let text = format_add_message("Alice", "uuid-123", "vless://uuid-123@1.2.3.4:443?...");
        assert!(text.contains("Alice"), "text: {}", text);
        assert!(text.contains("uuid-123"), "text: {}", text);
        assert!(text.contains("vless://"), "text: {}", text);
        assert!(text.contains("✅"), "text: {}", text);
    }

    #[test]
    fn test_format_add_message_special_name() {
        let text = format_add_message("Bob's Phone [iOS]", "uuid-456", "vless://...");
        assert!(text.contains("Bob's Phone [iOS]"), "text: {}", text);
    }

    #[test]
    fn test_format_delete_confirm_message() {
        let text = format_delete_confirm_message("Alice");
        assert!(text.contains("Alice"), "text: {}", text);
        assert!(text.contains("⚠️"), "text: {}", text);
        assert!(text.contains("Delete"), "text: {}", text);
    }

    #[test]
    fn test_format_delete_success_message() {
        let text = format_delete_success_message("Alice");
        assert!(text.contains("Alice"), "text: {}", text);
        assert!(text.contains("deleted"), "text: {}", text);
    }

    #[test]
    fn test_validate_user_name_valid() {
        assert!(validate_user_name("Alice").is_none());
        assert!(validate_user_name("Bob's Phone").is_none());
        assert!(validate_user_name("Admin [macOS]").is_none());
        assert!(validate_user_name("a").is_none());
    }

    #[test]
    fn test_validate_user_name_empty() {
        assert!(validate_user_name("").is_some());
        assert!(validate_user_name("   ").is_some());
    }

    #[test]
    fn test_validate_user_name_too_long() {
        let long_name = "a".repeat(65);
        let result = validate_user_name(&long_name);
        assert!(result.is_some());
        assert!(result.unwrap().contains("too long"));
    }

    #[test]
    fn test_delete_confirmation_keyboard() {
        let keyboard = delete_confirmation_keyboard("uuid-123");
        let buttons = &keyboard.inline_keyboard;
        assert_eq!(buttons.len(), 1, "should have one row");
        assert_eq!(buttons[0].len(), 2, "should have two buttons");

        // Check button text
        assert_eq!(buttons[0][0].text, "Yes, delete");
        assert_eq!(buttons[0][1].text, "Cancel");
    }

    #[test]
    fn test_delete_confirmation_keyboard_callback_data() {
        let keyboard = delete_confirmation_keyboard("test-uuid-abc");
        let buttons = &keyboard.inline_keyboard;

        // Extract callback data from buttons
        let confirm_data = match &buttons[0][0].kind {
            teloxide::types::InlineKeyboardButtonKind::CallbackData(d) => d.clone(),
            _ => panic!("expected callback data"),
        };
        let cancel_data = match &buttons[0][1].kind {
            teloxide::types::InlineKeyboardButtonKind::CallbackData(d) => d.clone(),
            _ => panic!("expected callback data"),
        };

        assert_eq!(confirm_data, "delete_confirm:test-uuid-abc");
        assert_eq!(cancel_data, "delete_cancel:test-uuid-abc");
    }

    #[test]
    fn test_validate_user_name_at_sign() {
        assert!(validate_user_name("foo@bar").is_some());
    }

    #[test]
    fn test_validate_user_name_triple_arrow() {
        assert!(validate_user_name("foo>>>bar").is_some());
    }

    #[test]
    fn test_validate_user_name_usage_hint() {
        let result = validate_user_name("").unwrap();
        assert!(result.contains("/add"), "result: {}", result);
    }

    // -- /url formatting tests --

    #[test]
    fn test_format_url_message() {
        let text = format_url_message("Alice", "vless://uuid@1.2.3.4:443?test=1#Alice");
        assert!(text.contains("Alice"), "text: {}", text);
        assert!(text.contains("vless://"), "text: {}", text);
        assert!(text.contains("🔗"), "text: {}", text);
    }

    #[test]
    fn test_format_url_message_special_name() {
        let text = format_url_message("Bob's Phone [iOS]", "vless://...");
        assert!(text.contains("Bob's Phone [iOS]"), "text: {}", text);
    }

    #[test]
    fn test_bot_commands_include_url_and_qr() {
        let cmds = Command::bot_commands();
        let descriptions: String = cmds
            .iter()
            .map(|c| c.command.as_str())
            .collect::<Vec<_>>()
            .join(",");
        assert!(descriptions.contains("url"), "commands: {}", descriptions);
        assert!(descriptions.contains("qr"), "commands: {}", descriptions);
    }

    // -- Inline keyboard / callback data tests --

    #[test]
    fn test_build_user_keyboard_url_prefix() {
        let users = vec![
            XrayUser {
                uuid: "uuid-1".to_string(),
                name: "Alice".to_string(),
                email: "Alice@vpn".to_string(),
                flow: String::new(),
                stats: TrafficStats::default(),
                online_count: 0,
            },
            XrayUser {
                uuid: "uuid-2".to_string(),
                name: "Bob".to_string(),
                email: "Bob@vpn".to_string(),
                flow: String::new(),
                stats: TrafficStats::default(),
                online_count: 0,
            },
        ];
        let keyboard = build_user_keyboard(&users, URL_PREFIX);
        let buttons = &keyboard.inline_keyboard;
        assert_eq!(buttons.len(), 2, "should have two rows");
        assert_eq!(buttons[0][0].text, "Alice");
        assert_eq!(buttons[1][0].text, "Bob");

        // Verify callback data
        let alice_data = match &buttons[0][0].kind {
            teloxide::types::InlineKeyboardButtonKind::CallbackData(d) => d.clone(),
            _ => panic!("expected callback data"),
        };
        let bob_data = match &buttons[1][0].kind {
            teloxide::types::InlineKeyboardButtonKind::CallbackData(d) => d.clone(),
            _ => panic!("expected callback data"),
        };
        assert_eq!(alice_data, "url:Alice");
        assert_eq!(bob_data, "url:Bob");
    }

    #[test]
    fn test_build_user_keyboard_skips_empty_names() {
        let users = vec![
            XrayUser {
                uuid: "uuid-1".to_string(),
                name: "Alice".to_string(),
                email: "Alice@vpn".to_string(),
                flow: String::new(),
                stats: TrafficStats::default(),
                online_count: 0,
            },
            XrayUser {
                uuid: "uuid-2".to_string(),
                name: String::new(),
                email: "@vpn".to_string(),
                flow: String::new(),
                stats: TrafficStats::default(),
                online_count: 0,
            },
        ];
        let keyboard = build_user_keyboard(&users, "url:");
        let buttons = &keyboard.inline_keyboard;
        assert_eq!(buttons.len(), 1, "should skip unnamed user");
        assert_eq!(buttons[0][0].text, "Alice");
    }

    #[test]
    fn test_build_user_keyboard_empty_list() {
        let keyboard = build_user_keyboard(&[], "url:");
        assert!(keyboard.inline_keyboard.is_empty());
    }

    #[test]
    fn test_build_user_keyboard_different_prefixes() {
        let users = vec![XrayUser {
            uuid: "uuid-1".to_string(),
            name: "Alice".to_string(),
            email: "Alice@vpn".to_string(),
            flow: String::new(),
            stats: TrafficStats::default(),
            online_count: 0,
        }];

        for prefix in &["url:", "qr:", "delete:"] {
            let keyboard = build_user_keyboard(&users, prefix);
            let data = match &keyboard.inline_keyboard[0][0].kind {
                teloxide::types::InlineKeyboardButtonKind::CallbackData(d) => d.clone(),
                _ => panic!("expected callback data"),
            };
            assert_eq!(data, format!("{}Alice", prefix));
        }
    }

    #[test]
    fn test_url_callback_data_parsing() {
        // Verify that URL_PREFIX correctly strips from callback data
        let callback_data = "url:Alice";
        let user_name = callback_data.strip_prefix(URL_PREFIX);
        assert_eq!(user_name, Some("Alice"));

        let callback_data = "url:Bob's Phone [iOS]";
        let user_name = callback_data.strip_prefix(URL_PREFIX);
        assert_eq!(user_name, Some("Bob's Phone [iOS]"));
    }

    #[test]
    fn test_url_callback_data_no_match() {
        let callback_data = "qr:Alice";
        assert!(callback_data.strip_prefix(URL_PREFIX).is_none());
    }

    #[test]
    fn test_is_admin_negative_id() {
        // Telegram group chat IDs can be negative
        let mut config = Config::default();
        config.telegram_admin_chat_id = Some(-100123456);
        assert!(is_admin(&config, ChatId(-100123456)));
        assert!(!is_admin(&config, ChatId(100123456)));
    }

    #[test]
    fn test_admin_id_from_cli() {
        use clap::Parser;
        let cli = crate::config::Cli::parse_from([
            "app",
            "--telegram-bot",
            "--local",
            "--admin-id",
            "123456789",
        ]);
        assert_eq!(cli.admin_id, Some(123456789));
    }

    #[test]
    fn test_admin_id_merged_into_config() {
        let mut config = Config::default();
        assert_eq!(config.telegram_admin_chat_id, None);

        use clap::Parser;
        let cli = crate::config::Cli::parse_from(["app", "--admin-id", "999"]);
        config.merge_cli(&cli);
        assert_eq!(config.telegram_admin_chat_id, Some(999));
    }
}
