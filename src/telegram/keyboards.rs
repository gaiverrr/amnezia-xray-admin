//! Inline keyboard builders and the selection-message wrappers that go with them.
//!
//! Each function is pure and testable without touching teloxide's runtime.

use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

use crate::xray::types::XrayUser;

use super::{DELETE_CANCEL_PREFIX, DELETE_CONFIRM_PREFIX};

/// Result of building an inline keyboard for user selection.
pub(super) struct UserKeyboardResult {
    pub keyboard: InlineKeyboardMarkup,
    pub skipped_names: Vec<String>,
    pub unnamed_count: usize,
}

/// Build an inline keyboard listing users, with each button using the given callback prefix.
///
/// Each button shows the user's name and has callback data `"{prefix}{name}"`.
/// Users with empty names or callback data exceeding Telegram's 64-byte limit are skipped.
/// Returns the keyboard markup, skipped names, and count of unnamed users.
pub(super) fn build_user_keyboard(users: &[XrayUser], callback_prefix: &str) -> UserKeyboardResult {
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
pub(super) fn format_selection_message(
    base_msg: &str,
    skipped: &[String],
    unnamed_count: usize,
) -> String {
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
pub(super) fn format_empty_keyboard_message(
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
pub(super) fn delete_confirmation_keyboard(user_uuid: &str) -> InlineKeyboardMarkup {
    let confirm = InlineKeyboardButton::callback(
        "Yes, delete",
        format!("{}{}", DELETE_CONFIRM_PREFIX, user_uuid),
    );
    let cancel =
        InlineKeyboardButton::callback("Cancel", format!("{}{}", DELETE_CANCEL_PREFIX, user_uuid));
    InlineKeyboardMarkup::new(vec![vec![confirm, cancel]])
}

#[cfg(test)]
mod tests {
    use super::super::{DELETE_PREFIX, QR_PREFIX, URL_PREFIX};
    use super::*;
    use crate::xray::types::TrafficStats;

    #[test]
    fn test_delete_confirmation_keyboard() {
        let keyboard = delete_confirmation_keyboard("uuid-123");
        let buttons = &keyboard.inline_keyboard;
        assert_eq!(buttons.len(), 1, "should have one row");
        assert_eq!(buttons[0].len(), 2, "should have two buttons");

        assert_eq!(buttons[0][0].text, "Yes, delete");
        assert_eq!(buttons[0][1].text, "Cancel");
    }

    #[test]
    fn test_delete_confirmation_keyboard_callback_data() {
        let keyboard = delete_confirmation_keyboard("test-uuid-abc");
        let buttons = &keyboard.inline_keyboard;

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
        let long_name = "a".repeat(61);
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
}
