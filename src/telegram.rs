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

/// Handle incoming bot commands.
async fn handle_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
    state: Arc<BotState>,
) -> ResponseResult<()> {
    let chat_id = msg.chat.id;

    let mut config = state.config.lock().await;

    if !is_admin_or_unset(&config, chat_id) {
        bot.send_message(chat_id, "Access denied. Only the admin can use this bot.")
            .await?;
        return Ok(());
    }

    match cmd {
        Command::Start => {
            let is_new = config.telegram_admin_chat_id.is_none();
            if is_new {
                config.telegram_admin_chat_id = Some(chat_id.0);
                if let Err(e) = config.save() {
                    log::error!("Failed to save admin chat ID: {}", e);
                }
                log::info!("Admin registered: chat_id={}", chat_id.0);
            }
            bot.send_message(chat_id, welcome_text(is_new)).await?;
        }
        Command::Help => {
            bot.send_message(chat_id, help_text()).await?;
        }
    }

    Ok(())
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
    }

    #[test]
    fn test_help_text_not_empty() {
        let text = help_text();
        assert!(!text.is_empty());
        assert!(text.lines().count() >= 5);
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
