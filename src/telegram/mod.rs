//! Telegram bot for managing Xray users via Telegram commands.
//!
//! Admin is set at deploy time via `--admin-id`. Only the configured admin
//! can interact with the bot; all other users get "Access denied".
//!
//! Module layout:
//! - `mod.rs` (this file) — bot entry point (`run_bot`), `BotState`,
//!   `Command` enum, admin check, `handle_command` / `handle_callback` dispatchers,
//!   shared callback-prefix constants.
//! - `format` — pure text formatters + input validator.
//! - `keyboards` — inline keyboard builders + their selection-message wrappers.
//! - `handlers` — async `cmd_*` functions (one per command).

mod format;
mod handlers;
mod keyboards;

use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{BotCommandScope, ChatId, InputFile, ParseMode, Recipient};
use teloxide::utils::command::BotCommands;
use tokio::sync::Mutex;

use crate::backend_trait::XrayBackend;
use crate::config::Config;
use crate::error::Result;
use crate::xray::client::XrayClient;

use format::{access_denied_text, help_text, validate_user_name, welcome_text};
use handlers::{
    cmd_add, cmd_delete_execute, cmd_delete_prompt, cmd_qr, cmd_status, cmd_url, cmd_user_keyboard,
    cmd_users,
};
use keyboards::{format_empty_keyboard_message, format_selection_message};

/// State shared across all Telegram handlers.
pub(crate) struct BotState {
    pub backend: Box<dyn XrayBackend>,
    pub config: Mutex<Config>,
}

/// Callback query prefix for URL inline buttons.
pub(super) const URL_PREFIX: &str = "url:";
/// Callback query prefix for QR inline buttons.
pub(super) const QR_PREFIX: &str = "qr:";
/// Callback query prefix for delete user selection buttons.
pub(super) const DELETE_PREFIX: &str = "delete:";
/// Callback query prefix for delete confirmation buttons.
pub(super) const DELETE_CONFIRM_PREFIX: &str = "delete_confirm:";
/// Callback query prefix for delete cancel buttons.
pub(super) const DELETE_CANCEL_PREFIX: &str = "delete_cancel:";

/// Commands recognized by the bot.
#[derive(BotCommands, Clone, Debug)]
#[command(rename_rule = "lowercase")]
pub(super) enum Command {
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
                        match crate::xray::url::render_qr_png(&vless_url) {
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
    fn test_bot_commands_parse() {
        let cmds = Command::bot_commands();
        let descriptions: String = cmds
            .iter()
            .map(|c| c.command.as_str())
            .collect::<Vec<_>>()
            .join(",");
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

    #[test]
    fn test_url_callback_data_parsing() {
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
    fn test_delete_callback_data_parsing() {
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
        let confirm_data = "delete_confirm:uuid-123";
        assert!(confirm_data.strip_prefix(DELETE_PREFIX).is_none());

        let cancel_data = "delete_cancel:uuid-123";
        assert!(cancel_data.strip_prefix(DELETE_PREFIX).is_none());
    }

    #[test]
    fn test_delete_confirm_cancel_prefixes_distinct() {
        assert_ne!(DELETE_PREFIX, DELETE_CONFIRM_PREFIX);
        assert_ne!(DELETE_PREFIX, DELETE_CANCEL_PREFIX);
        assert_ne!(DELETE_CONFIRM_PREFIX, DELETE_CANCEL_PREFIX);

        assert!(!DELETE_CONFIRM_PREFIX.starts_with(DELETE_PREFIX));
        assert!(!DELETE_CANCEL_PREFIX.starts_with(DELETE_PREFIX));
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
        let state = BotState {
            backend: Box::new(NoopBackend {
                host: "1.2.3.4".to_string(),
            }),
            config: Mutex::new(Config::default()),
        };
        assert_eq!(state.backend.hostname(), "1.2.3.4");
    }
}
