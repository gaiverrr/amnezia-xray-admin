//! Plain-text formatters for bot replies and small input validators.
//!
//! Everything in here is pure (no async, no backend); unit-testable in
//! isolation. The dispatcher in `mod.rs` and handlers in `handlers.rs`
//! call these to build message bodies.

use crate::xray::types::{TrafficStats, XrayUser};

/// Minimal summary of the running xray instance, used by `/status`.
#[derive(Debug, Clone, Default)]
pub(super) struct ServerInfo {
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

/// Format a duration in seconds as a compact "Xd Yh Zm" string.
pub(super) fn format_uptime(seconds: u64) -> String {
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

/// Format the /help response text.
pub(super) fn help_text() -> String {
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
pub(super) fn welcome_text() -> String {
    [
        "Welcome, admin!",
        "",
        "Use /help to see available commands.",
    ]
    .join("\n")
}

/// Format the access denied message for non-admin users.
pub(super) fn access_denied_text() -> String {
    "Access denied. Contact the server administrator.".to_string()
}

/// Format the /users response: list users with traffic stats.
pub(super) fn format_users_message(users: &[(XrayUser, TrafficStats, u32)]) -> String {
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

/// Minimal HTML escape for Telegram HTML parse mode (only & < > need escaping
/// outside of attributes).
pub(super) fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Formatted add-user message. Uses Telegram HTML parse mode so that UUID and
/// URL appear inside <code>…</code> blocks, which most clients render as
/// tap-to-copy. Caller must `.parse_mode(ParseMode::Html)` when sending.
pub(super) fn format_add_message(name: &str, uuid: &str, vless_url: &str) -> String {
    [
        format!("✅ User '{}' added.", html_escape(name)),
        String::new(),
        format!("UUID: <code>{}</code>", html_escape(uuid)),
        format!("URL: <code>{}</code>", html_escape(vless_url)),
    ]
    .join("\n")
}

/// Format the /delete confirmation prompt.
pub(super) fn format_delete_confirm_message(name: &str) -> String {
    format!(
        "⚠️ Delete user '{}'?\n\nThis will revoke their access immediately.",
        name
    )
}

/// Format the /delete success response.
pub(super) fn format_delete_success_message(name: &str) -> String {
    format!("🗑 User '{}' deleted.", name)
}

/// Validate a user name for `/add` command. UX-level gate only — shell-safety
/// (rejecting `'`, `"`, `\`, control chars) is enforced downstream by
/// `validate_name` inside `XrayClient::{add_client,remove_client}`. Do not
/// remove the downstream check thinking this one covers it.
pub(super) fn validate_user_name(name: &str) -> Option<String> {
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

/// Format the /status response: server info and online users summary.
pub(super) fn format_status_message(
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

/// Format the /url response: vless:// URL wrapped in <code> so Telegram
/// renders it as a tap-to-copy block. Caller must send with ParseMode::Html.
pub(super) fn format_url_message(name: &str, vless_url: &str) -> String {
    format!(
        "🔗 {} URL:\n\n<code>{}</code>",
        html_escape(name),
        html_escape(vless_url)
    )
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
        assert!(text.contains("🟢 1"), "text: {}", text);
        assert!(text.contains("⚪"), "text: {}", text);
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
}
