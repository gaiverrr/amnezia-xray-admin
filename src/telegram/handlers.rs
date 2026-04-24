//! Per-command async handlers — the ones the dispatcher in `mod.rs` calls
//! once admin access has been verified. Each `cmd_*` returns a
//! plain value (text, URL, QR bytes) and lets the dispatcher do the
//! Telegram-specific sending.

use teloxide::types::InlineKeyboardMarkup;

use crate::backend_trait::XrayBackend;
use crate::xray::client::XrayClient;
use crate::xray::types::{TrafficStats, XrayUser};
use crate::xray::url::{render_qr_png, render_xhttp_url, XhttpUrlParams};

use super::format::{
    format_add_message, format_delete_confirm_message, format_delete_success_message,
    format_status_message, format_uptime, format_url_message, format_users_message, html_escape,
    ServerInfo,
};
use super::keyboards::{build_user_keyboard, delete_confirmation_keyboard, UserKeyboardResult};
use super::BotState;

/// Execute /users command: list users with stats.
pub(super) async fn cmd_users(
    state: &BotState,
) -> std::result::Result<String, crate::error::AppError> {
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
pub(super) async fn cmd_add(
    state: &BotState,
    name: &str,
) -> std::result::Result<(String, String), crate::error::AppError> {
    let client = XrayClient::new(state.backend.as_ref());
    let entry = client.add_client(name).await?;
    let url = build_bridge_url_for(state, name).await?;
    Ok((format_add_message(name, &entry.uuid, &url), url))
}

/// Execute /delete prompt: find user and return confirmation message with inline keyboard.
pub(super) async fn cmd_delete_prompt(
    state: &BotState,
    name: &str,
) -> std::result::Result<(String, InlineKeyboardMarkup), crate::error::AppError> {
    let client = XrayClient::new(state.backend.as_ref());
    let uuid = client.get_uuid(name).await?;
    let text = format_delete_confirm_message(name);
    let keyboard = delete_confirmation_keyboard(&uuid);
    Ok((text, keyboard))
}

/// Execute the actual user deletion by UUID.
pub(super) async fn cmd_delete_execute(
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
pub(super) async fn cmd_user_keyboard(
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
pub(super) async fn cmd_url(
    state: &BotState,
    name: &str,
) -> std::result::Result<String, crate::error::AppError> {
    let vless_url = build_bridge_url_for(state, name).await?;
    Ok(format_url_message(name, &vless_url))
}

/// Execute /qr command: generate QR code PNG for a user's vless URL.
pub(super) async fn cmd_qr(
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

/// Execute /status command: server info + online summary.
pub(super) async fn cmd_status(
    state: &BotState,
) -> std::result::Result<String, crate::error::AppError> {
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
