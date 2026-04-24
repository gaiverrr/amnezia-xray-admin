//! Telegram bot for managing Xray users via Telegram commands.
//!
//! Admin is set at deploy time via `--admin-id`. Only the configured admin
//! can interact with the bot; all other users get "Access denied".

use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{
    BotCommandScope, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, InputFile, ParseMode,
    Recipient,
};
use teloxide::utils::command::BotCommands;
use tokio::sync::Mutex;

use crate::backend_trait::XrayBackend;
use crate::config::Config;
use crate::error::Result;
use crate::xray::client::XrayClient;
use crate::xray::types::{TrafficStats, XrayUser};
use crate::xray::url::{render_qr_png, render_xhttp_url, XhttpUrlParams};

/// Minimal summary of the running xray instance, used by `/status`.
#[derive(Debug, Clone, Default)]
pub struct ServerInfo {
    pub version: String,
    pub uplink: u64,
    pub downlink: u64,
}

/// Format a byte count as a human-readable string (e.g. `1.5 MB`).
fn format_bytes(n: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{} {}", n, UNITS[0])
    } else {
        format!("{:.1} {}", v, UNITS[i])
    }
}

/// State shared across all Telegram handlers.
pub struct BotState {
    pub backend: Box<dyn XrayBackend>,
    pub config: Mutex<Config>,
}

/// Callback query prefix for URL inline buttons.
const URL_PREFIX: &str = "url:";
/// Callback query prefix for QR inline buttons.
const QR_PREFIX: &str = "qr:";
/// Callback query prefix for delete user selection buttons.
const DELETE_PREFIX: &str = "delete:";
/// Callback query prefix for delete confirmation buttons.
const DELETE_CONFIRM_PREFIX: &str = "delete_confirm:";
/// Callback query prefix for delete cancel buttons.
const DELETE_CANCEL_PREFIX: &str = "delete_cancel:";

/// Commands recognized by the bot.
#[derive(BotCommands, Clone, Debug)]
#[command(rename_rule = "lowercase")]
pub enum Command {
    #[command(description = "Welcome message")]
    Start,
    #[command(description = "Show help")]
    Help,
    #[command(description = "List users with stats")]
    Users,
    #[command(description = "Server info + online users")]
    Status,
    #[command(description = "Add a new user: /add <name>")]
    Add(String),
    #[command(description = "Delete a user: /delete <name>")]
    Delete(String),
    #[command(description = "Get vless:// URL: /url <name>")]
    Url(String),
    #[command(description = "Get QR code: /qr <name>")]
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
    [
        "Welcome, admin!",
        "",
        "Use /help to see available commands.",
    ]
    .join("\n")
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
/// Minimal HTML escape for Telegram HTML parse mode (only & < > need escaping
/// outside of attributes).
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Formatted add-user message. Uses Telegram HTML parse mode so that UUID and
/// URL appear inside <code>…</code> blocks, which most clients render as
/// tap-to-copy. Caller must `.parse_mode(ParseMode::Html)` when sending.
pub fn format_add_message(name: &str, uuid: &str, vless_url: &str) -> String {
    [
        format!("✅ User '{}' added.", html_escape(name)),
        String::new(),
        format!("UUID: <code>{}</code>", html_escape(uuid)),
        format!("URL: <code>{}</code>", html_escape(vless_url)),
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

/// Validate a user name for `/add` command. UX-level gate only — shell-safety
/// (rejecting `'`, `"`, `\`, control chars) is enforced downstream by
/// `validate_name` inside `XrayClient::{add_client,remove_client}`. Do not
/// remove the downstream check thinking this one covers it.
pub fn validate_user_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Some("Usage: /add <name>".to_string());
    }
    // Telegram callback_data has a 64-byte limit. With the longest prefix
    // "delete:" (7 bytes), names must stay under 57 bytes. Use 50 for margin.
    if trimmed.len() > 50 {
        return Some("Name too long (max 50 bytes).".to_string());
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Some("Name must not contain control characters.".to_string());
    }
    if trimmed.contains('@') || trimmed.contains(">>>") {
        return Some("Name must not contain '@' or '>>>'.".to_string());
    }
    None
}

/// Result of building an inline keyboard for user selection.
pub struct UserKeyboardResult {
    pub keyboard: InlineKeyboardMarkup,
    pub skipped_names: Vec<String>,
    pub unnamed_count: usize,
}

/// Build an inline keyboard listing users, with each button using the given callback prefix.
///
/// Each button shows the user's name and has callback data `"{prefix}{name}"`.
/// Users with empty names or callback data exceeding Telegram's 64-byte limit are skipped.
/// Returns the keyboard markup, skipped names, and count of unnamed users.
pub fn build_user_keyboard(users: &[XrayUser], callback_prefix: &str) -> UserKeyboardResult {
    /// Telegram's maximum callback_data size in bytes.
    const MAX_CALLBACK_DATA_BYTES: usize = 64;

    let mut skipped_names: Vec<String> = Vec::new();
    let unnamed_count = users.iter().filter(|u| u.name.is_empty()).count();

    let buttons: Vec<Vec<InlineKeyboardButton>> = users
        .iter()
        .filter(|u| !u.name.is_empty())
        .filter(|u| {
            if callback_prefix.len() + u.name.len() > MAX_CALLBACK_DATA_BYTES {
                skipped_names.push(u.name.clone());
                false
            } else {
                true
            }
        })
        .map(|u| {
            vec![InlineKeyboardButton::callback(
                &u.name,
                format!("{}{}", callback_prefix, u.name),
            )]
        })
        .collect();
    UserKeyboardResult {
        keyboard: InlineKeyboardMarkup::new(buttons),
        skipped_names,
        unnamed_count,
    }
}

/// Format a truncated list of skipped user names, keeping the total under a safe length.
/// Shows as many names as fit, then "and N more" if truncated.
fn format_skipped_names(skipped: &[String], max_bytes: usize) -> String {
    if skipped.is_empty() {
        return String::new();
    }
    let mut result = String::new();
    for (shown, name) in skipped.iter().enumerate() {
        let separator = if shown == 0 { "" } else { ", " };
        let remaining = skipped.len() - shown;
        let suffix = format!(" and {} more", remaining - 1);
        let needed = separator.len() + name.len();
        let budget_with_suffix = if remaining > 1 {
            needed + suffix.len()
        } else {
            needed
        };
        if result.len() + budget_with_suffix > max_bytes {
            if result.is_empty() {
                // First name already exceeds budget — truncate it
                let ellipsis = "...";
                let avail = max_bytes.saturating_sub(ellipsis.len());
                // Truncate at a char boundary
                let truncated = &name[..name.floor_char_boundary(avail)];
                return format!("{}{}", truncated, ellipsis);
            }
            result.push_str(&format!(" and {} more", remaining));
            return result;
        }
        result.push_str(separator);
        result.push_str(name);
    }
    result
}

/// Format a user-selection prompt, appending notes about skipped or unnamed users if any.
fn format_selection_message(base_msg: &str, skipped: &[String], unnamed_count: usize) -> String {
    let mut notes = Vec::new();
    if !skipped.is_empty() {
        let template_overhead = base_msg.len() + 100;
        let max_names = 3500usize.saturating_sub(template_overhead);
        let names = format_skipped_names(skipped, max_names);
        notes.push(format!(
            "{} user(s) have names too long for inline buttons. Use the command with the name directly: {}",
            skipped.len(),
            names
        ));
    }
    if unnamed_count > 0 {
        notes.push(format!(
            "{} unnamed user(s) not shown. Use /users to see them.",
            unnamed_count
        ));
    }
    if notes.is_empty() {
        base_msg.to_string()
    } else {
        format!("{}\n\nNote: {}", base_msg, notes.join("\n"))
    }
}

/// Format the message when no inline buttons are available.
fn format_empty_keyboard_message(
    command_hint: &str,
    skipped: &[String],
    unnamed_count: usize,
) -> String {
    let mut parts = Vec::new();
    if !skipped.is_empty() {
        let names = format_skipped_names(skipped, 3900);
        parts.push(format!(
            "{} user(s) have names too long for inline buttons. Use {} directly for: {}",
            skipped.len(),
            command_hint,
            names
        ));
    }
    if unnamed_count > 0 {
        parts.push(format!(
            "{} user(s) have no name set. Use /users to see them.",
            unnamed_count
        ));
    }
    if parts.is_empty() {
        "No users found.".to_string()
    } else {
        parts.join("\n\n")
    }
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
    uptime: &str,
    latest_version: Option<&str>,
) -> String {
    let mut lines = Vec::new();
    lines.push("📊 Server Status:".to_string());
    lines.push(String::new());

    let version_line = match latest_version {
        Some(latest) if latest != server_info.version => {
            format!("Xray: v{} (⬆️ v{} available)", server_info.version, latest)
        }
        Some(_) => format!("Xray: v{} ✅", server_info.version),
        None => format!("Xray: v{}", server_info.version),
    };
    lines.push(version_line);

    if !uptime.is_empty() {
        lines.push(format!("Uptime: {}", uptime));
    }
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
                match cmd_add(&state, &name).await {
                    Ok((text, vless_url)) => {
                        bot.send_message(chat_id, text)
                            .parse_mode(ParseMode::Html)
                            .await?;
                        // Render + send QR as a photo. Best-effort: if QR
                        // encoding fails the user still has the URL.
                        match render_qr_png(&vless_url) {
                            Ok(png) => {
                                let input = InputFile::memory(png).file_name("qr.png");
                                bot.send_photo(chat_id, input).await?;
                            }
                            Err(e) => {
                                bot.send_message(chat_id, format!("(QR render failed: {})", e))
                                    .await?;
                            }
                        }
                        // Defer xray reload until after the response is sent
                        // so the bot's own HTTP proxy (which runs through
                        // xray) stays up during the reply. Now that responses
                        // have been sent, reload so xray picks up the new
                        // client.
                        if let Err(e) = XrayClient::new(state.backend.as_ref()).reload_xray().await
                        {
                            log::warn!("reload_xray after /add failed: {}", e);
                        }
                    }
                    Err(e) => {
                        bot.send_message(chat_id, format!("Error: {}", e)).await?;
                    }
                }
            }
        }
        Command::Delete(name) => {
            let name = name.trim().to_string();
            if name.is_empty() {
                match cmd_user_keyboard(&state, DELETE_PREFIX).await {
                    Ok(result) => {
                        if result.keyboard.inline_keyboard.is_empty() {
                            let msg = format_empty_keyboard_message(
                                "/delete <name>",
                                &result.skipped_names,
                                result.unnamed_count,
                            );
                            bot.send_message(chat_id, msg).await?;
                        } else {
                            let msg = format_selection_message(
                                "Select a user to delete:",
                                &result.skipped_names,
                                result.unnamed_count,
                            );
                            bot.send_message(chat_id, msg)
                                .reply_markup(result.keyboard)
                                .await?;
                        }
                    }
                    Err(e) => {
                        bot.send_message(chat_id, format!("Error: {}", e)).await?;
                    }
                }
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
                    Ok(result) => {
                        if result.keyboard.inline_keyboard.is_empty() {
                            let msg = format_empty_keyboard_message(
                                "/url <name>",
                                &result.skipped_names,
                                result.unnamed_count,
                            );
                            bot.send_message(chat_id, msg).await?;
                        } else {
                            let msg = format_selection_message(
                                "Select a user:",
                                &result.skipped_names,
                                result.unnamed_count,
                            );
                            bot.send_message(chat_id, msg)
                                .reply_markup(result.keyboard)
                                .await?;
                        }
                    }
                    Err(e) => {
                        bot.send_message(chat_id, format!("Error: {}", e)).await?;
                    }
                }
            } else {
                match cmd_url(&state, &name).await {
                    Ok(t) => {
                        bot.send_message(chat_id, t)
                            .parse_mode(ParseMode::Html)
                            .await?;
                    }
                    Err(e) => {
                        bot.send_message(chat_id, format!("Error: {}", e)).await?;
                    }
                }
            }
        }
        Command::Qr(name) => {
            let name = name.trim().to_string();
            if name.is_empty() {
                match cmd_user_keyboard(&state, QR_PREFIX).await {
                    Ok(result) => {
                        if result.keyboard.inline_keyboard.is_empty() {
                            let msg = format_empty_keyboard_message(
                                "/qr <name>",
                                &result.skipped_names,
                                result.unnamed_count,
                            );
                            bot.send_message(chat_id, msg).await?;
                        } else {
                            let msg = format_selection_message(
                                "Select a user:",
                                &result.skipped_names,
                                result.unnamed_count,
                            );
                            bot.send_message(chat_id, msg)
                                .reply_markup(result.keyboard)
                                .await?;
                        }
                    }
                    Err(e) => {
                        bot.send_message(chat_id, format!("Error: {}", e)).await?;
                    }
                }
            } else {
                match cmd_qr(&state, &name).await {
                    Ok((png_bytes, caption)) => {
                        let input = InputFile::memory(png_bytes).file_name("qr.png");
                        bot.send_photo(chat_id, input)
                            .caption(caption)
                            .parse_mode(ParseMode::Html)
                            .await?;
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
    let client = XrayClient::new(state.backend.as_ref());
    let clients = client.list_clients().await?;
    let stats_map = client.get_all_user_stats().await;
    let user_data: Vec<(XrayUser, TrafficStats, u32)> = clients
        .into_iter()
        .map(|c| {
            let name = c.email.strip_suffix("@vpn").unwrap_or(&c.email).to_string();
            let stats = stats_map.get(&c.email).cloned().unwrap_or_default();
            let user = XrayUser {
                uuid: c.uuid,
                name,
                email: c.email.clone(),
                flow: String::new(),
                stats: stats.clone(),
                online_count: 0,
            };
            (user, stats, 0)
        })
        .collect();
    Ok(format_users_message(&user_data))
}

/// Execute /add command: add a user, return (message text, vless URL) so the
/// caller can both display the text and render a QR for the URL.
async fn cmd_add(
    state: &BotState,
    name: &str,
) -> std::result::Result<(String, String), crate::error::AppError> {
    let client = XrayClient::new(state.backend.as_ref());
    let entry = client.add_client(name).await?;
    let url = build_bridge_url_for(state, name).await?;
    Ok((format_add_message(name, &entry.uuid, &url), url))
}

/// Execute /delete prompt: find user and return confirmation message with inline keyboard.
async fn cmd_delete_prompt(
    state: &BotState,
    name: &str,
) -> std::result::Result<(String, InlineKeyboardMarkup), crate::error::AppError> {
    let client = XrayClient::new(state.backend.as_ref());
    // Verify the user exists before prompting for confirmation.
    let uuid = client.get_uuid(name).await?;
    let text = format_delete_confirm_message(name);
    let keyboard = delete_confirmation_keyboard(&uuid);
    Ok((text, keyboard))
}

/// Execute the actual user deletion by UUID.
async fn cmd_delete_execute(
    state: &BotState,
    uuid: &str,
) -> std::result::Result<String, crate::error::AppError> {
    let client = XrayClient::new(state.backend.as_ref());
    let clients = client.list_clients().await?;
    let entry = clients.iter().find(|c| c.uuid == uuid).ok_or_else(|| {
        crate::error::AppError::Xray(format!("user with uuid '{}' not found", uuid))
    })?;
    let name = entry
        .email
        .strip_suffix("@vpn")
        .unwrap_or(&entry.email)
        .to_string();
    client.remove_client(&name).await?;
    Ok(format_delete_success_message(&name))
}

/// Build an inline keyboard listing all users, for use by /url, /qr, /delete without arguments.
async fn cmd_user_keyboard(
    state: &BotState,
    callback_prefix: &str,
) -> std::result::Result<UserKeyboardResult, crate::error::AppError> {
    let client = XrayClient::new(state.backend.as_ref());
    let clients = client.list_clients().await?;
    let users: Vec<XrayUser> = clients
        .into_iter()
        .map(|c| {
            let name = c.email.strip_suffix("@vpn").unwrap_or(&c.email).to_string();
            XrayUser {
                uuid: c.uuid,
                name,
                email: c.email,
                flow: String::new(),
                stats: TrafficStats::default(),
                online_count: 0,
            }
        })
        .collect();
    Ok(build_user_keyboard(&users, callback_prefix))
}

/// Build the vless URL for `name` given the current bridge config.
async fn build_bridge_url_for(
    state: &BotState,
    name: &str,
) -> std::result::Result<String, crate::error::AppError> {
    let client = XrayClient::new(state.backend.as_ref());
    let uuid = client.get_uuid(name).await?;
    let params = client.bridge_public_params().await?;
    Ok(render_xhttp_url(&XhttpUrlParams {
        uuid,
        host: state.backend.hostname().to_string(),
        port: params.port,
        path: params.path,
        sni: params.sni,
        public_key: params.public_key,
        short_id: params.short_id,
        name: name.to_string(),
    }))
}

/// Execute /url command: get vless:// URL for a user.
async fn cmd_url(
    state: &BotState,
    name: &str,
) -> std::result::Result<String, crate::error::AppError> {
    let vless_url = build_bridge_url_for(state, name).await?;
    Ok(format_url_message(name, &vless_url))
}

/// Execute /qr command: generate QR code PNG for a user's vless URL.
async fn cmd_qr(
    state: &BotState,
    name: &str,
) -> std::result::Result<(Vec<u8>, String), crate::error::AppError> {
    let vless_url = build_bridge_url_for(state, name).await?;
    let png_bytes = render_qr_png(&vless_url)
        .map_err(|e| crate::error::AppError::Xray(format!("QR generation failed: {}", e)))?;
    let caption = format!(
        "🔗 {}\n\n<code>{}</code>",
        html_escape(name),
        html_escape(&vless_url)
    );
    Ok((png_bytes, caption))
}

/// Format the /url response: vless:// URL wrapped in <code> so Telegram
/// renders it as a tap-to-copy block. Caller must send with ParseMode::Html.
pub fn format_url_message(name: &str, vless_url: &str) -> String {
    format!(
        "🔗 {} URL:\n\n<code>{}</code>",
        html_escape(name),
        html_escape(vless_url)
    )
}

/// Handle callback queries from inline keyboard buttons (e.g., delete confirmation).
async fn handle_callback(bot: Bot, q: CallbackQuery, state: Arc<BotState>) -> ResponseResult<()> {
    let data = match q.data {
        Some(ref d) => d.as_str(),
        None => return Ok(()),
    };

    // Use q.from.id for the admin check (the user who pressed the button),
    // not q.message.chat().id (which would be the group ID in group chats).
    let caller_id = ChatId(q.from.id.0 as i64);
    let chat_id = match q.message {
        Some(ref msg) => msg.chat().id,
        None => return Ok(()),
    };

    // Check admin access using caller_id (who pressed the button)
    {
        let config = state.config.lock().await;
        if !is_admin(&config, caller_id) {
            bot.answer_callback_query(q.id.clone())
                .text("Access denied.")
                .await?;
            return Ok(());
        }
    }

    if let Some(user_name) = data.strip_prefix(URL_PREFIX) {
        let result = cmd_url(&state, user_name).await;
        bot.answer_callback_query(q.id.clone()).await?;
        if let Some(ref msg) = q.message {
            match result {
                Ok(t) => {
                    bot.edit_message_text(chat_id, msg.id(), &t)
                        .parse_mode(ParseMode::Html)
                        .await?;
                }
                Err(e) => {
                    bot.edit_message_text(chat_id, msg.id(), format!("Error: {}", e))
                        .await?;
                }
            }
        }
    } else if let Some(user_name) = data.strip_prefix(QR_PREFIX) {
        bot.answer_callback_query(q.id.clone()).await?;
        match cmd_qr(&state, user_name).await {
            Ok((png_bytes, caption)) => {
                let input = InputFile::memory(png_bytes).file_name("qr.png");
                bot.send_photo(chat_id, input)
                    .caption(caption)
                    .parse_mode(ParseMode::Html)
                    .await?;
                // Remove the inline keyboard from the original message
                if let Some(ref msg) = q.message {
                    bot.edit_message_text(chat_id, msg.id(), format!("QR for {}", user_name))
                        .await?;
                }
            }
            Err(e) => {
                bot.send_message(chat_id, format!("Error: {}", e)).await?;
            }
        }
    } else if let Some(user_name) = data.strip_prefix(DELETE_PREFIX) {
        // User selected from /delete inline keyboard — show confirmation
        match cmd_delete_prompt(&state, user_name).await {
            Ok((text, keyboard)) => {
                bot.answer_callback_query(q.id.clone()).await?;
                if let Some(ref msg) = q.message {
                    bot.edit_message_text(chat_id, msg.id(), &text)
                        .reply_markup(keyboard)
                        .await?;
                }
            }
            Err(e) => {
                bot.answer_callback_query(q.id.clone())
                    .text(format!("Error: {}", e))
                    .await?;
            }
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
        // Defer xray reload until after the response is sent; see cmd_add.
        if let Err(e) = XrayClient::new(state.backend.as_ref()).reload_xray().await {
            log::warn!("reload_xray after /delete failed: {}", e);
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
    let client = XrayClient::new(state.backend.as_ref());
    let clients = client.list_clients().await?;
    let (uplink, downlink) = client.get_inbound_stats("client-in").await;

    let version = fetch_trimmed(
        &*state.backend,
        "xray version 2>/dev/null | head -n 1 | awk '{print $2}'",
    )
    .await
    .unwrap_or_else(|| "unknown".to_string());

    let uptime = fetch_xray_uptime(&*state.backend).await.unwrap_or_default();

    let server_info = ServerInfo {
        version,
        uplink,
        downlink,
    };
    Ok(format_status_message(
        &server_info,
        clients.len(),
        0,
        &uptime,
        None,
    ))
}

/// Run a shell command and return trimmed stdout on success, else None.
async fn fetch_trimmed(backend: &dyn XrayBackend, cmd: &str) -> Option<String> {
    let out = backend.exec_on_host(cmd).await.ok()?;
    if !out.success() {
        return None;
    }
    let trimmed = out.stdout.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Fetch xray.service uptime as a human-readable string (e.g. "3h 12m"),
/// computed from systemd's `ActiveEnterTimestampMonotonic` and the host's
/// monotonic clock. Returns None on failure.
async fn fetch_xray_uptime(backend: &dyn XrayBackend) -> Option<String> {
    // One shell invocation: read the service's active-since timestamp and
    // the uptime_in_seconds value, then print their delta.
    let cmd = "\
        enter_us=$(systemctl show xray --property=ActiveEnterTimestampMonotonic --value 2>/dev/null); \
        now_us=$(awk '{printf \"%d\", $1 * 1000000}' /proc/uptime 2>/dev/null); \
        if [ -n \"$enter_us\" ] && [ \"$enter_us\" != \"0\" ] && [ -n \"$now_us\" ]; then \
            echo $(( (now_us - enter_us) / 1000000 )); \
        fi";
    let seconds_str = fetch_trimmed(backend, cmd).await?;
    let seconds: u64 = seconds_str.parse().ok()?;
    Some(format_uptime(seconds))
}

/// Format a duration in seconds as a compact "Xd Yh Zm" string.
fn format_uptime(seconds: u64) -> String {
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3600;
    let minutes = (seconds % 3600) / 60;
    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

/// Start the Telegram bot and block until shutdown.
pub async fn run_bot(token: &str, backend: Box<dyn XrayBackend>, config: Config) -> Result<()> {
    log::info!("Starting Telegram bot...");

    let bot = Bot::new(token);

    // Register public commands visible to all users
    let public_cmds = vec![
        teloxide::types::BotCommand::new("start", "Welcome message"),
        teloxide::types::BotCommand::new("help", "Show help"),
    ];
    if let Err(e) = bot.set_my_commands(public_cmds).await {
        log::warn!("Failed to register public bot commands: {}", e);
    }

    // Register full command list scoped to admin chat only
    if let Some(admin_id) = config.telegram_admin_chat_id {
        let scope = BotCommandScope::Chat {
            chat_id: Recipient::Id(ChatId(admin_id)),
        };
        if let Err(e) = bot
            .set_my_commands(Command::bot_commands())
            .scope(scope)
            .await
        {
            log::warn!("Failed to register admin bot commands: {}", e);
        }
    }

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
    fn test_format_uptime_ranges() {
        assert_eq!(format_uptime(0), "0m");
        assert_eq!(format_uptime(59), "0m");
        assert_eq!(format_uptime(60), "1m");
        assert_eq!(format_uptime(3_600), "1h 0m");
        assert_eq!(format_uptime(3_660), "1h 1m");
        assert_eq!(format_uptime(86_400), "1d 0h 0m");
        assert_eq!(format_uptime(90_061), "1d 1h 1m");
    }

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
        let config = Config {
            telegram_admin_chat_id: Some(12345),
            ..Default::default()
        };
        assert!(is_admin(&config, ChatId(12345)));
    }

    #[test]
    fn test_is_admin_wrong_user() {
        let config = Config {
            telegram_admin_chat_id: Some(12345),
            ..Default::default()
        };
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
        let text = format_status_message(&info, 5, 2, "Up 2 hours", Some("1.8.4"));
        assert!(text.contains("v1.8.4"), "text: {}", text);
        assert!(text.contains("✅"), "text: {}", text);
        assert!(text.contains("Up 2 hours"), "text: {}", text);
        assert!(text.contains("5"), "text: {}", text);
        assert!(text.contains("2 online"), "text: {}", text);
        assert!(text.contains("500.0 MB"), "text: {}", text);
        assert!(text.contains("10.0 GB"), "text: {}", text);
    }

    #[test]
    fn test_format_status_message_update_available() {
        let info = ServerInfo {
            version: "1.8.0".to_string(),
            uplink: 0,
            downlink: 0,
        };
        let text = format_status_message(&info, 3, 0, "Up 5 minutes", Some("1.9.0"));
        assert!(text.contains("⬆️ v1.9.0"), "text: {}", text);
    }

    #[test]
    fn test_format_status_message_zero_online() {
        let info = ServerInfo {
            version: "1.8.0".to_string(),
            uplink: 0,
            downlink: 0,
        };
        let text = format_status_message(&info, 3, 0, "", None);
        assert!(text.contains("0 online"), "text: {}", text);
        assert!(text.contains("3"), "text: {}", text);
    }

    #[test]
    fn test_config_telegram_admin_serialization() {
        let config = Config {
            telegram_admin_chat_id: Some(123456789),
            ..Default::default()
        };
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
    fn test_add_without_argument_shows_usage_hint() {
        // /add without argument passes empty string to validate_user_name,
        // which returns a usage hint instead of proceeding with add
        let result = validate_user_name("");
        assert_eq!(result, Some("Usage: /add <name>".to_string()));

        let result_whitespace = validate_user_name("   ");
        assert_eq!(result_whitespace, Some("Usage: /add <name>".to_string()));
    }

    #[test]
    fn test_validate_user_name_too_long() {
        let long_name = "a".repeat(51);
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
        let result = build_user_keyboard(&users, URL_PREFIX);
        assert!(result.skipped_names.is_empty());
        assert_eq!(result.unnamed_count, 0);
        let buttons = &result.keyboard.inline_keyboard;
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
        let result = build_user_keyboard(&users, "url:");
        let buttons = &result.keyboard.inline_keyboard;
        assert_eq!(buttons.len(), 1, "should skip unnamed user");
        assert_eq!(buttons[0][0].text, "Alice");
        assert_eq!(result.unnamed_count, 1);
    }

    #[test]
    fn test_build_user_keyboard_empty_list() {
        let result = build_user_keyboard(&[], "url:");
        assert!(result.keyboard.inline_keyboard.is_empty());
        assert!(result.skipped_names.is_empty());
        assert_eq!(result.unnamed_count, 0);
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
            let result = build_user_keyboard(&users, prefix);
            let data = match &result.keyboard.inline_keyboard[0][0].kind {
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
    fn test_qr_callback_data_parsing() {
        let callback_data = "qr:Alice";
        let user_name = callback_data.strip_prefix(QR_PREFIX);
        assert_eq!(user_name, Some("Alice"));

        let callback_data = "qr:Bob's Phone [iOS]";
        let user_name = callback_data.strip_prefix(QR_PREFIX);
        assert_eq!(user_name, Some("Bob's Phone [iOS]"));
    }

    #[test]
    fn test_qr_callback_data_no_match() {
        let callback_data = "url:Alice";
        assert!(callback_data.strip_prefix(QR_PREFIX).is_none());

        let callback_data = "delete:Alice";
        assert!(callback_data.strip_prefix(QR_PREFIX).is_none());
    }

    #[test]
    fn test_build_user_keyboard_qr_prefix() {
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
        let result = build_user_keyboard(&users, QR_PREFIX);
        assert!(result.skipped_names.is_empty());
        let buttons = &result.keyboard.inline_keyboard;
        assert_eq!(buttons.len(), 2);
        assert_eq!(buttons[0][0].text, "Alice");
        assert_eq!(buttons[1][0].text, "Bob");

        let alice_data = match &buttons[0][0].kind {
            teloxide::types::InlineKeyboardButtonKind::CallbackData(d) => d.clone(),
            _ => panic!("expected callback data"),
        };
        let bob_data = match &buttons[1][0].kind {
            teloxide::types::InlineKeyboardButtonKind::CallbackData(d) => d.clone(),
            _ => panic!("expected callback data"),
        };
        assert_eq!(alice_data, "qr:Alice");
        assert_eq!(bob_data, "qr:Bob");
    }

    #[test]
    fn test_is_admin_negative_id() {
        // Telegram group chat IDs can be negative
        let config = Config {
            telegram_admin_chat_id: Some(-100123456),
            ..Default::default()
        };
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

    // -- /delete inline buttons tests --

    #[test]
    fn test_build_user_keyboard_delete_prefix() {
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
        let result = build_user_keyboard(&users, DELETE_PREFIX);
        assert!(result.skipped_names.is_empty());
        let buttons = &result.keyboard.inline_keyboard;
        assert_eq!(buttons.len(), 2);
        assert_eq!(buttons[0][0].text, "Alice");
        assert_eq!(buttons[1][0].text, "Bob");

        let alice_data = match &buttons[0][0].kind {
            teloxide::types::InlineKeyboardButtonKind::CallbackData(d) => d.clone(),
            _ => panic!("expected callback data"),
        };
        let bob_data = match &buttons[1][0].kind {
            teloxide::types::InlineKeyboardButtonKind::CallbackData(d) => d.clone(),
            _ => panic!("expected callback data"),
        };
        assert_eq!(alice_data, "delete:Alice");
        assert_eq!(bob_data, "delete:Bob");
    }

    #[test]
    fn test_delete_callback_data_parsing() {
        // Verify DELETE_PREFIX correctly strips from callback data
        let callback_data = "delete:Alice";
        let user_name = callback_data.strip_prefix(DELETE_PREFIX);
        assert_eq!(user_name, Some("Alice"));

        let callback_data = "delete:Bob's Phone [iOS]";
        let user_name = callback_data.strip_prefix(DELETE_PREFIX);
        assert_eq!(user_name, Some("Bob's Phone [iOS]"));
    }

    #[test]
    fn test_delete_callback_data_no_match() {
        let callback_data = "url:Alice";
        assert!(callback_data.strip_prefix(DELETE_PREFIX).is_none());

        let callback_data = "qr:Alice";
        assert!(callback_data.strip_prefix(DELETE_PREFIX).is_none());
    }

    #[test]
    fn test_delete_prefix_does_not_match_confirm_cancel() {
        // "delete:" should NOT match "delete_confirm:" or "delete_cancel:"
        let confirm_data = "delete_confirm:uuid-123";
        assert!(confirm_data.strip_prefix(DELETE_PREFIX).is_none());

        let cancel_data = "delete_cancel:uuid-123";
        assert!(cancel_data.strip_prefix(DELETE_PREFIX).is_none());
    }

    #[test]
    fn test_delete_confirm_cancel_prefixes_distinct() {
        // Verify all delete-related prefixes are distinct
        assert_ne!(DELETE_PREFIX, DELETE_CONFIRM_PREFIX);
        assert_ne!(DELETE_PREFIX, DELETE_CANCEL_PREFIX);
        assert_ne!(DELETE_CONFIRM_PREFIX, DELETE_CANCEL_PREFIX);

        // None is a prefix of another (important for strip_prefix correctness)
        assert!(!DELETE_CONFIRM_PREFIX.starts_with(DELETE_PREFIX));
        assert!(!DELETE_CANCEL_PREFIX.starts_with(DELETE_PREFIX));
    }

    #[test]
    fn test_build_user_keyboard_skips_long_names() {
        // "delete:" is 7 bytes, so max name is 57 bytes
        let long_name = "a".repeat(58);
        let short_name = "Alice".to_string();
        let users = vec![
            XrayUser {
                uuid: "uuid-1".to_string(),
                name: short_name.clone(),
                email: "Alice@vpn".to_string(),
                flow: String::new(),
                stats: TrafficStats::default(),
                online_count: 0,
            },
            XrayUser {
                uuid: "uuid-2".to_string(),
                name: long_name.clone(),
                email: "long@vpn".to_string(),
                flow: String::new(),
                stats: TrafficStats::default(),
                online_count: 0,
            },
        ];
        let result = build_user_keyboard(&users, DELETE_PREFIX);
        assert_eq!(result.keyboard.inline_keyboard.len(), 1);
        assert_eq!(result.keyboard.inline_keyboard[0][0].text, "Alice");
        assert_eq!(result.skipped_names, vec![long_name]);
    }

    #[test]
    fn test_build_user_keyboard_all_skipped() {
        let long_name = "a".repeat(61); // exceeds 64 bytes with "url:" prefix
        let users = vec![XrayUser {
            uuid: "uuid-1".to_string(),
            name: long_name.clone(),
            email: "long@vpn".to_string(),
            flow: String::new(),
            stats: TrafficStats::default(),
            online_count: 0,
        }];
        let result = build_user_keyboard(&users, URL_PREFIX);
        assert!(result.keyboard.inline_keyboard.is_empty());
        assert_eq!(result.skipped_names, vec![long_name]);
    }

    #[test]
    fn test_build_user_keyboard_all_unnamed() {
        let users = vec![
            XrayUser {
                uuid: "uuid-1".to_string(),
                name: String::new(),
                email: "@vpn".to_string(),
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
        let result = build_user_keyboard(&users, URL_PREFIX);
        assert!(result.keyboard.inline_keyboard.is_empty());
        assert!(result.skipped_names.is_empty());
        assert_eq!(result.unnamed_count, 2);
    }

    #[test]
    fn test_format_empty_keyboard_message_no_users() {
        let msg = format_empty_keyboard_message("/url <name>", &[], 0);
        assert_eq!(msg, "No users found.");
    }

    #[test]
    fn test_format_empty_keyboard_message_unnamed_only() {
        let msg = format_empty_keyboard_message("/url <name>", &[], 3);
        assert!(msg.contains("3 user(s) have no name set"));
        assert!(msg.contains("/users"));
    }

    #[test]
    fn test_format_empty_keyboard_message_skipped_and_unnamed() {
        let skipped = vec!["LongName".to_string()];
        let msg = format_empty_keyboard_message("/delete <name>", &skipped, 2);
        assert!(msg.contains("1 user(s) have names too long"));
        assert!(msg.contains("LongName"));
        assert!(msg.contains("2 user(s) have no name set"));
    }

    #[test]
    fn test_format_selection_message_no_skipped() {
        let msg = format_selection_message("Select a user:", &[], 0);
        assert_eq!(msg, "Select a user:");
    }

    #[test]
    fn test_format_selection_message_with_skipped() {
        let skipped = vec!["LongUserName".to_string()];
        let msg = format_selection_message("Select a user:", &skipped, 0);
        assert!(msg.contains("Select a user:"));
        assert!(msg.contains("LongUserName"));
        assert!(msg.contains("1 user(s)"));
    }

    #[test]
    fn test_format_selection_message_with_unnamed() {
        let msg = format_selection_message("Select a user:", &[], 3);
        assert!(msg.contains("Select a user:"));
        assert!(msg.contains("3 unnamed user(s) not shown"));
        assert!(msg.contains("/users"));
    }

    #[test]
    fn test_format_selection_message_with_skipped_and_unnamed() {
        let skipped = vec!["LongName".to_string()];
        let msg = format_selection_message("Select a user:", &skipped, 2);
        assert!(msg.contains("Select a user:"));
        assert!(msg.contains("LongName"));
        assert!(msg.contains("2 unnamed user(s) not shown"));
    }

    /// Minimal no-op `XrayBackend` used to construct a `BotState` in tests.
    struct NoopBackend {
        host: String,
    }

    #[async_trait::async_trait]
    impl XrayBackend for NoopBackend {
        async fn exec_in_container(
            &self,
            _cmd: &str,
        ) -> crate::error::Result<crate::ssh::CommandOutput> {
            Ok(crate::ssh::CommandOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            })
        }
        async fn exec_on_host(
            &self,
            _cmd: &str,
        ) -> crate::error::Result<crate::ssh::CommandOutput> {
            Ok(crate::ssh::CommandOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            })
        }
        fn container_name(&self) -> &str {
            ""
        }
        fn hostname(&self) -> &str {
            &self.host
        }
    }

    #[test]
    fn test_bot_state_constructible() {
        // BotState should be constructible with the minimum set of fields
        // required by bridge-mode handlers.
        let state = BotState {
            backend: Box::new(NoopBackend {
                host: "1.2.3.4".to_string(),
            }),
            config: Mutex::new(Config::default()),
        };
        assert_eq!(state.backend.hostname(), "1.2.3.4");
    }
}
