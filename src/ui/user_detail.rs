use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::dashboard::format_bytes;
use super::theme;
use crate::xray::types::XrayUser;

/// Sub-mode within the user detail view
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailMode {
    /// Normal view showing user info
    View,
    /// Delete confirmation — user must type the name to confirm
    DeleteConfirm,
    /// Deletion succeeded
    DeleteSuccess,
    /// Deletion failed
    DeleteError(String),
}

/// State for the user detail panel
#[derive(Debug, Clone)]
pub struct UserDetailState {
    /// The user being viewed (snapshot from when detail was opened)
    pub user: Option<XrayUser>,
    /// Online IPs for this user
    pub online_ips: Vec<String>,
    /// Current mode
    pub mode: DetailMode,
    /// Text typed into the delete confirmation input
    pub delete_input: String,
    /// Whether deletion has been confirmed (for async processing)
    pub delete_confirmed: bool,
    /// Whether clipboard copy was attempted
    pub clipboard_copied: bool,
}

impl Default for UserDetailState {
    fn default() -> Self {
        Self {
            user: None,
            online_ips: Vec::new(),
            mode: DetailMode::View,
            delete_input: String::new(),
            delete_confirmed: false,
            clipboard_copied: false,
        }
    }
}

impl UserDetailState {
    /// Initialize detail view for a user
    pub fn open(&mut self, user: XrayUser) {
        self.user = Some(user);
        self.online_ips.clear();
        self.mode = DetailMode::View;
        self.delete_input.clear();
        self.delete_confirmed = false;
        self.clipboard_copied = false;
    }

    /// Reset state when leaving detail view
    pub fn close(&mut self) {
        self.user = None;
        self.online_ips.clear();
        self.mode = DetailMode::View;
        self.delete_input.clear();
        self.delete_confirmed = false;
        self.clipboard_copied = false;
    }

    /// Handle key input. Returns true if the event was consumed.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        match &self.mode {
            DetailMode::View => self.handle_view_key(key),
            DetailMode::DeleteConfirm => self.handle_delete_confirm_key(key),
            DetailMode::DeleteSuccess | DetailMode::DeleteError(_) => {
                // Any key returns to dashboard (handled by app.rs)
                false
            }
        }
    }

    fn handle_view_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('d') => {
                self.mode = DetailMode::DeleteConfirm;
                self.delete_input.clear();
                true
            }
            KeyCode::Char('c') => {
                self.clipboard_copied = true;
                true
            }
            // q and Esc handled by app.rs for navigation
            _ => false,
        }
    }

    fn handle_delete_confirm_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.mode = DetailMode::View;
                self.delete_input.clear();
                true
            }
            KeyCode::Char(c) => {
                self.delete_input.push(c);
                true
            }
            KeyCode::Backspace => {
                self.delete_input.pop();
                true
            }
            KeyCode::Enter => {
                if let Some(ref user) = self.user {
                    if self.delete_input == user.name {
                        self.delete_confirmed = true;
                    }
                }
                true
            }
            _ => true,
        }
    }

    /// Check if delete was confirmed and consume the flag
    pub fn take_delete_confirmed(&mut self) -> bool {
        std::mem::replace(&mut self.delete_confirmed, false)
    }

    /// Check if clipboard copy was requested and consume the flag
    pub fn take_clipboard_copied(&mut self) -> bool {
        std::mem::replace(&mut self.clipboard_copied, false)
    }

    /// Set deletion result
    pub fn set_delete_success(&mut self) {
        self.mode = DetailMode::DeleteSuccess;
    }

    pub fn set_delete_error(&mut self, msg: String) {
        self.mode = DetailMode::DeleteError(msg);
    }

    /// Get the user name if available
    pub fn user_name(&self) -> Option<&str> {
        self.user.as_ref().map(|u| u.name.as_str())
    }

    /// Check if delete input matches the user name
    pub fn delete_input_matches(&self) -> bool {
        self.user
            .as_ref()
            .map(|u| self.delete_input == u.name)
            .unwrap_or(false)
    }
}

/// Write OSC 52 escape sequence to copy text to clipboard.
/// This works in terminals that support OSC 52 (iTerm2, kitty, tmux, etc.)
pub fn osc52_copy(text: &str) -> String {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    format!("\x1b]52;c;{}\x07", encoded)
}

/// Draw the user detail view
pub fn draw(state: &UserDetailState, frame: &mut ratatui::Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border_style())
        .title(Span::styled(" User Detail ", theme::secondary_style()))
        .style(theme::text_style());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let user = match &state.user {
        Some(u) => u,
        None => {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "  No user selected",
                    theme::muted_style(),
                ))),
                inner,
            );
            return;
        }
    };

    match &state.mode {
        DetailMode::View => draw_view(
            user,
            &state.online_ips,
            state.clipboard_copied,
            frame,
            inner,
        ),
        DetailMode::DeleteConfirm => draw_delete_confirm(user, &state.delete_input, frame, inner),
        DetailMode::DeleteSuccess => draw_delete_success(user, frame, inner),
        DetailMode::DeleteError(msg) => draw_delete_error(user, msg, frame, inner),
    }
}

fn draw_view(
    user: &XrayUser,
    online_ips: &[String],
    _clipboard_copied: bool,
    frame: &mut ratatui::Frame,
    area: Rect,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // spacer
            Constraint::Length(1), // name
            Constraint::Length(1), // uuid
            Constraint::Length(1), // email
            Constraint::Length(1), // spacer
            Constraint::Length(1), // traffic header
            Constraint::Length(1), // upload
            Constraint::Length(1), // download
            Constraint::Length(1), // spacer
            Constraint::Length(1), // online header
            Constraint::Length(1), // online count
            Constraint::Min(1),    // online IPs + hints
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  Name:  ", theme::muted_style()),
            Span::styled(&user.name, theme::title_style()),
        ])),
        chunks[1],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  UUID:  ", theme::muted_style()),
            Span::styled(&user.uuid, theme::secondary_style()),
        ])),
        chunks[2],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  Email: ", theme::muted_style()),
            Span::styled(&user.email, theme::secondary_style()),
        ])),
        chunks[3],
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  Traffic",
            theme::secondary_style(),
        ))),
        chunks[5],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("    Upload:   ", theme::muted_style()),
            Span::styled(format_bytes(user.stats.uplink), theme::text_style()),
        ])),
        chunks[6],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("    Download: ", theme::muted_style()),
            Span::styled(format_bytes(user.stats.downlink), theme::text_style()),
        ])),
        chunks[7],
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  Online Status",
            theme::secondary_style(),
        ))),
        chunks[9],
    );

    let online_style = if user.online_count > 0 {
        theme::title_style()
    } else {
        theme::alert_style()
    };
    let online_text = if user.online_count > 0 {
        format!("\u{25cf} Online ({})", user.online_count)
    } else {
        "\u{25cf} Offline".to_string()
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("    ", theme::muted_style()),
            Span::styled(online_text, online_style),
        ])),
        chunks[10],
    );

    // Online IPs + keybinding hints in remaining space
    let remaining = chunks[11];
    let ip_lines_count = online_ips.len().min(5) as u16;
    let remaining_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(ip_lines_count + 1), // IPs area
            Constraint::Min(0),                     // spacer
            Constraint::Length(1),                  // keybind hints
        ])
        .split(remaining);

    let mut ip_lines: Vec<Line> = Vec::new();
    if !online_ips.is_empty() {
        ip_lines.push(Line::from(Span::styled(
            "    Connected IPs:",
            theme::muted_style(),
        )));
        for ip in online_ips.iter().take(5) {
            ip_lines.push(Line::from(vec![
                Span::styled("      ", theme::muted_style()),
                Span::styled(ip.as_str(), theme::secondary_style()),
            ]));
        }
        if online_ips.len() > 5 {
            ip_lines.push(Line::from(Span::styled(
                format!("      ... and {} more", online_ips.len() - 5),
                theme::muted_style(),
            )));
        }
    }
    frame.render_widget(Paragraph::new(ip_lines), remaining_chunks[0]);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  [Esc] back  [d]elete  [c]opy URL  [q] QR code",
            theme::muted_style(),
        ))),
        remaining_chunks[2],
    );
}

fn draw_delete_confirm(
    user: &XrayUser,
    delete_input: &str,
    frame: &mut ratatui::Frame,
    area: Rect,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // spacer
            Constraint::Length(1), // warning
            Constraint::Length(1), // spacer
            Constraint::Length(1), // instruction
            Constraint::Length(1), // user name to type
            Constraint::Length(1), // spacer
            Constraint::Length(1), // input
            Constraint::Length(1), // spacer
            Constraint::Length(1), // hint
            Constraint::Min(0),
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  WARNING: You are about to delete this user permanently!",
            theme::alert_style(),
        ))),
        chunks[1],
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  Type the user name to confirm deletion:",
            theme::secondary_style(),
        ))),
        chunks[3],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  Expected: ", theme::muted_style()),
            Span::styled(&user.name, theme::title_style()),
        ])),
        chunks[4],
    );

    let input_style = if delete_input == user.name {
        theme::title_style()
    } else {
        theme::alert_style()
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  > ", theme::muted_style()),
            Span::styled(format!("{}_", delete_input), input_style),
        ])),
        chunks[6],
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  [Enter] confirm  [Esc] cancel",
            theme::muted_style(),
        ))),
        chunks[8],
    );
}

fn draw_delete_success(user: &XrayUser, frame: &mut ratatui::Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // spacer
            Constraint::Length(1), // message
            Constraint::Length(1), // user info
            Constraint::Length(2), // spacer
            Constraint::Length(1), // hint
            Constraint::Min(0),
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  User deleted successfully.",
            theme::title_style(),
        ))),
        chunks[1],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  Removed: ", theme::muted_style()),
            Span::styled(&user.name, theme::secondary_style()),
        ])),
        chunks[2],
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  Press any key to return to dashboard",
            theme::muted_style(),
        ))),
        chunks[4],
    );
}

fn draw_delete_error(user: &XrayUser, msg: &str, frame: &mut ratatui::Frame, area: Rect) {
    let _ = user; // user context available if needed
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // spacer
            Constraint::Length(1), // error label
            Constraint::Length(1), // error msg
            Constraint::Length(2), // spacer
            Constraint::Length(1), // hint
            Constraint::Min(0),
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  Failed to delete user:",
            theme::alert_style(),
        ))),
        chunks[1],
    );

    let truncated = if msg.len() > 60 { &msg[..60] } else { msg };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("  {}", truncated),
            theme::muted_style(),
        ))),
        chunks[2],
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  Press any key to return to dashboard",
            theme::muted_style(),
        ))),
        chunks[4],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xray::types::TrafficStats;
    use crossterm::event::{KeyEventKind, KeyEventState, KeyModifiers};

    fn make_key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn make_test_user() -> XrayUser {
        XrayUser {
            uuid: "aaaaaaaa-1111-2222-3333-bbbbbbbbbbbb".to_string(),
            name: "alice".to_string(),
            email: "alice@vpn".to_string(),
            flow: "xtls-rprx-vision".to_string(),
            stats: TrafficStats {
                uplink: 1024 * 1024,
                downlink: 5 * 1024 * 1024,
            },
            online_count: 2,
        }
    }

    #[test]
    fn test_default_state() {
        let state = UserDetailState::default();
        assert!(state.user.is_none());
        assert!(state.online_ips.is_empty());
        assert_eq!(state.mode, DetailMode::View);
        assert_eq!(state.delete_input, "");
        assert!(!state.delete_confirmed);
        assert!(!state.clipboard_copied);
    }

    #[test]
    fn test_open_sets_user() {
        let mut state = UserDetailState::default();
        let user = make_test_user();
        state.open(user.clone());
        assert_eq!(state.user.as_ref().unwrap().name, "alice");
        assert_eq!(state.mode, DetailMode::View);
    }

    #[test]
    fn test_open_resets_previous_state() {
        let mut state = UserDetailState::default();
        state.delete_input = "leftover".to_string();
        state.mode = DetailMode::DeleteConfirm;
        state.delete_confirmed = true;
        state.clipboard_copied = true;
        state.online_ips = vec!["1.2.3.4".to_string()];

        state.open(make_test_user());
        assert_eq!(state.delete_input, "");
        assert_eq!(state.mode, DetailMode::View);
        assert!(!state.delete_confirmed);
        assert!(!state.clipboard_copied);
        assert!(state.online_ips.is_empty());
    }

    #[test]
    fn test_close_clears_state() {
        let mut state = UserDetailState::default();
        state.open(make_test_user());
        state.online_ips = vec!["1.2.3.4".to_string()];
        state.close();
        assert!(state.user.is_none());
        assert!(state.online_ips.is_empty());
        assert_eq!(state.mode, DetailMode::View);
    }

    #[test]
    fn test_view_d_enters_delete_confirm() {
        let mut state = UserDetailState::default();
        state.open(make_test_user());
        let consumed = state.handle_key(make_key(KeyCode::Char('d')));
        assert!(consumed);
        assert_eq!(state.mode, DetailMode::DeleteConfirm);
        assert_eq!(state.delete_input, "");
    }

    #[test]
    fn test_view_c_sets_clipboard_flag() {
        let mut state = UserDetailState::default();
        state.open(make_test_user());
        let consumed = state.handle_key(make_key(KeyCode::Char('c')));
        assert!(consumed);
        assert!(state.clipboard_copied);
    }

    #[test]
    fn test_view_q_not_consumed() {
        let mut state = UserDetailState::default();
        state.open(make_test_user());
        let consumed = state.handle_key(make_key(KeyCode::Char('q')));
        assert!(!consumed); // app.rs handles navigation
    }

    #[test]
    fn test_view_esc_not_consumed() {
        let mut state = UserDetailState::default();
        state.open(make_test_user());
        let consumed = state.handle_key(make_key(KeyCode::Esc));
        assert!(!consumed);
    }

    #[test]
    fn test_delete_confirm_typing() {
        let mut state = UserDetailState::default();
        state.open(make_test_user());
        state.mode = DetailMode::DeleteConfirm;

        state.handle_key(make_key(KeyCode::Char('a')));
        state.handle_key(make_key(KeyCode::Char('l')));
        state.handle_key(make_key(KeyCode::Char('i')));
        assert_eq!(state.delete_input, "ali");
    }

    #[test]
    fn test_delete_confirm_backspace() {
        let mut state = UserDetailState::default();
        state.open(make_test_user());
        state.mode = DetailMode::DeleteConfirm;
        state.delete_input = "alic".to_string();

        state.handle_key(make_key(KeyCode::Backspace));
        assert_eq!(state.delete_input, "ali");
    }

    #[test]
    fn test_delete_confirm_esc_returns_to_view() {
        let mut state = UserDetailState::default();
        state.open(make_test_user());
        state.mode = DetailMode::DeleteConfirm;
        state.delete_input = "ali".to_string();

        let consumed = state.handle_key(make_key(KeyCode::Esc));
        assert!(consumed);
        assert_eq!(state.mode, DetailMode::View);
        assert_eq!(state.delete_input, "");
    }

    #[test]
    fn test_delete_confirm_enter_with_matching_name() {
        let mut state = UserDetailState::default();
        state.open(make_test_user());
        state.mode = DetailMode::DeleteConfirm;
        state.delete_input = "alice".to_string();

        state.handle_key(make_key(KeyCode::Enter));
        assert!(state.delete_confirmed);
    }

    #[test]
    fn test_delete_confirm_enter_with_wrong_name() {
        let mut state = UserDetailState::default();
        state.open(make_test_user());
        state.mode = DetailMode::DeleteConfirm;
        state.delete_input = "bob".to_string();

        state.handle_key(make_key(KeyCode::Enter));
        assert!(!state.delete_confirmed);
    }

    #[test]
    fn test_delete_confirm_enter_empty() {
        let mut state = UserDetailState::default();
        state.open(make_test_user());
        state.mode = DetailMode::DeleteConfirm;

        state.handle_key(make_key(KeyCode::Enter));
        assert!(!state.delete_confirmed);
    }

    #[test]
    fn test_take_delete_confirmed() {
        let mut state = UserDetailState::default();
        state.delete_confirmed = true;
        assert!(state.take_delete_confirmed());
        assert!(!state.delete_confirmed); // consumed
        assert!(!state.take_delete_confirmed()); // already consumed
    }

    #[test]
    fn test_take_clipboard_copied() {
        let mut state = UserDetailState::default();
        state.clipboard_copied = true;
        assert!(state.take_clipboard_copied());
        assert!(!state.clipboard_copied);
        assert!(!state.take_clipboard_copied());
    }

    #[test]
    fn test_set_delete_success() {
        let mut state = UserDetailState::default();
        state.open(make_test_user());
        state.set_delete_success();
        assert_eq!(state.mode, DetailMode::DeleteSuccess);
    }

    #[test]
    fn test_set_delete_error() {
        let mut state = UserDetailState::default();
        state.open(make_test_user());
        state.set_delete_error("connection lost".to_string());
        assert_eq!(
            state.mode,
            DetailMode::DeleteError("connection lost".to_string())
        );
    }

    #[test]
    fn test_delete_success_keys_not_consumed() {
        let mut state = UserDetailState::default();
        state.open(make_test_user());
        state.mode = DetailMode::DeleteSuccess;
        assert!(!state.handle_key(make_key(KeyCode::Enter)));
        assert!(!state.handle_key(make_key(KeyCode::Esc)));
    }

    #[test]
    fn test_delete_error_keys_not_consumed() {
        let mut state = UserDetailState::default();
        state.open(make_test_user());
        state.mode = DetailMode::DeleteError("fail".to_string());
        assert!(!state.handle_key(make_key(KeyCode::Enter)));
    }

    #[test]
    fn test_user_name() {
        let mut state = UserDetailState::default();
        assert!(state.user_name().is_none());
        state.open(make_test_user());
        assert_eq!(state.user_name(), Some("alice"));
    }

    #[test]
    fn test_delete_input_matches() {
        let mut state = UserDetailState::default();
        assert!(!state.delete_input_matches()); // no user

        state.open(make_test_user());
        assert!(!state.delete_input_matches()); // empty input

        state.delete_input = "bob".to_string();
        assert!(!state.delete_input_matches()); // wrong name

        state.delete_input = "alice".to_string();
        assert!(state.delete_input_matches()); // correct
    }

    #[test]
    fn test_osc52_copy() {
        let result = osc52_copy("hello");
        assert!(result.starts_with("\x1b]52;c;"));
        assert!(result.ends_with("\x07"));
        // "hello" in base64 is "aGVsbG8="
        assert!(result.contains("aGVsbG8="));
    }

    #[test]
    fn test_osc52_copy_empty() {
        let result = osc52_copy("");
        assert!(result.starts_with("\x1b]52;c;"));
        assert!(result.ends_with("\x07"));
    }

    #[test]
    fn test_osc52_copy_unicode() {
        let result = osc52_copy("vless://uuid@host:443#test");
        assert!(result.starts_with("\x1b]52;c;"));
        assert!(result.ends_with("\x07"));
    }
}
