use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::theme;

/// Maximum allowed length for a user name
const MAX_NAME_LENGTH: usize = 32;

/// Check if a character is valid for a user name.
/// Only allows alphanumeric, hyphen, and underscore to prevent shell injection.
fn is_valid_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

/// Result of an add-user operation
#[derive(Debug, Clone)]
pub enum AddUserResult {
    None,
    Success { name: String, uuid: String },
    Error(String),
}

/// State for the add-user dialog
#[derive(Debug, Clone)]
pub struct AddUserState {
    pub name: String,
    pub result: AddUserResult,
    pub confirmed: bool,
    /// Cached vless:// URL after successful add
    pub cached_vless_url: Option<String>,
}

impl Default for AddUserState {
    fn default() -> Self {
        Self {
            name: String::new(),
            result: AddUserResult::None,
            confirmed: false,
            cached_vless_url: None,
        }
    }
}

impl AddUserState {
    /// Reset state for a new dialog opening
    pub fn reset(&mut self) {
        self.name.clear();
        self.result = AddUserResult::None;
        self.confirmed = false;
        self.cached_vless_url = None;
    }

    /// Handle a key event, returns true if the event was consumed
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        // If showing a result, any key dismisses
        if matches!(
            self.result,
            AddUserResult::Success { .. } | AddUserResult::Error(_)
        ) {
            return false; // let app.rs handle Esc/Enter to go back
        }

        match key.code {
            KeyCode::Char(c) => {
                // Only allow safe characters to prevent shell injection
                if is_valid_name_char(c) && self.name.len() < MAX_NAME_LENGTH {
                    self.name.push(c);
                }
                true
            }
            KeyCode::Backspace => {
                self.name.pop();
                true
            }
            KeyCode::Enter => {
                if !self.name.trim().is_empty() {
                    self.confirmed = true;
                }
                true
            }
            _ => false,
        }
    }

    /// Set the result after an add-user attempt
    pub fn set_success(&mut self, name: String, uuid: String) {
        self.result = AddUserResult::Success { name, uuid };
        self.confirmed = false;
    }

    /// Set an error result
    pub fn set_error(&mut self, msg: String) {
        self.result = AddUserResult::Error(msg);
        self.confirmed = false;
    }

    /// Check if the user has confirmed and name is valid
    pub fn is_confirmed(&self) -> bool {
        self.confirmed && !self.name.trim().is_empty()
    }
}

/// Compute a centered rect of given width/height within the area
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

/// Draw the add-user dialog as a modal overlay
pub fn draw(state: &AddUserState, frame: &mut ratatui::Frame, area: Rect) {
    match &state.result {
        AddUserResult::Success { name, uuid } => {
            draw_success(frame, area, name, uuid);
        }
        AddUserResult::Error(msg) => {
            draw_error(frame, area, msg);
        }
        AddUserResult::None => {
            draw_input(state, frame, area);
        }
    }
}

fn draw_input(state: &AddUserState, frame: &mut ratatui::Frame, area: Rect) {
    let dialog = centered_rect(50, 9, area);
    frame.render_widget(Clear, dialog);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::accent_style())
        .title(Span::styled(" Add User ", theme::accent_style()))
        .style(theme::text_style());

    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // spacer
            Constraint::Length(1), // prompt
            Constraint::Length(1), // spacer
            Constraint::Length(1), // input
            Constraint::Length(1), // spacer
            Constraint::Length(1), // hint
            Constraint::Min(0),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  Enter username for the new VPN client:",
            theme::secondary_style(),
        ))),
        rows[1],
    );

    let input_display = format!("  > {}_", state.name);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            input_display,
            theme::title_style(),
        ))),
        rows[3],
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  [Enter] confirm  [Esc] cancel",
            theme::muted_style(),
        ))),
        rows[5],
    );
}

fn draw_success(frame: &mut ratatui::Frame, area: Rect, name: &str, uuid: &str) {
    let dialog = centered_rect(56, 11, area);
    frame.render_widget(Clear, dialog);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::title_style())
        .title(Span::styled(" User Added ", theme::title_style()))
        .style(theme::text_style());

    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // spacer
            Constraint::Length(1), // success msg
            Constraint::Length(1), // spacer
            Constraint::Length(1), // name
            Constraint::Length(1), // uuid
            Constraint::Length(1), // spacer
            Constraint::Length(1), // hint
            Constraint::Min(0),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  User created successfully!",
            theme::title_style(),
        ))),
        rows[1],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  Name: ", theme::muted_style()),
            Span::styled(name, theme::secondary_style()),
        ])),
        rows[3],
    );

    let uuid_display = if uuid.len() > 36 { &uuid[..36] } else { uuid };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  UUID: ", theme::muted_style()),
            Span::styled(uuid_display, theme::secondary_style()),
        ])),
        rows[4],
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  [Esc] back  [q] QR code",
            theme::muted_style(),
        ))),
        rows[6],
    );
}

fn draw_error(frame: &mut ratatui::Frame, area: Rect, msg: &str) {
    let dialog = centered_rect(56, 9, area);
    frame.render_widget(Clear, dialog);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::alert_style())
        .title(Span::styled(" Error ", theme::alert_style()))
        .style(theme::text_style());

    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // spacer
            Constraint::Length(1), // error label
            Constraint::Length(1), // spacer
            Constraint::Length(1), // error msg
            Constraint::Length(1), // spacer
            Constraint::Length(1), // hint
            Constraint::Min(0),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  Failed to add user:",
            theme::alert_style(),
        ))),
        rows[1],
    );

    let truncated_msg = match msg.char_indices().nth(50) {
        Some((i, _)) => &msg[..i],
        None => msg,
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("  {}", truncated_msg),
            theme::muted_style(),
        ))),
        rows[3],
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  [Esc] back",
            theme::muted_style(),
        ))),
        rows[5],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState, KeyModifiers};

    fn make_key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn test_default_state() {
        let state = AddUserState::default();
        assert_eq!(state.name, "");
        assert!(!state.confirmed);
        assert!(matches!(state.result, AddUserResult::None));
    }

    #[test]
    fn test_reset() {
        let mut state = AddUserState::default();
        state.name = "alice".to_string();
        state.confirmed = true;
        state.result = AddUserResult::Success {
            name: "alice".to_string(),
            uuid: "123".to_string(),
        };
        state.reset();
        assert_eq!(state.name, "");
        assert!(!state.confirmed);
        assert!(matches!(state.result, AddUserResult::None));
    }

    #[test]
    fn test_typing_name() {
        let mut state = AddUserState::default();
        state.handle_key(make_key(KeyCode::Char('a')));
        state.handle_key(make_key(KeyCode::Char('l')));
        state.handle_key(make_key(KeyCode::Char('i')));
        state.handle_key(make_key(KeyCode::Char('c')));
        state.handle_key(make_key(KeyCode::Char('e')));
        assert_eq!(state.name, "alice");
    }

    #[test]
    fn test_backspace() {
        let mut state = AddUserState::default();
        state.name = "alice".to_string();
        state.handle_key(make_key(KeyCode::Backspace));
        assert_eq!(state.name, "alic");
    }

    #[test]
    fn test_backspace_empty() {
        let mut state = AddUserState::default();
        state.handle_key(make_key(KeyCode::Backspace));
        assert_eq!(state.name, "");
    }

    #[test]
    fn test_enter_confirms_with_name() {
        let mut state = AddUserState::default();
        state.name = "alice".to_string();
        state.handle_key(make_key(KeyCode::Enter));
        assert!(state.confirmed);
        assert!(state.is_confirmed());
    }

    #[test]
    fn test_enter_does_not_confirm_empty() {
        let mut state = AddUserState::default();
        state.handle_key(make_key(KeyCode::Enter));
        assert!(!state.confirmed);
        assert!(!state.is_confirmed());
    }

    #[test]
    fn test_enter_does_not_confirm_whitespace_only() {
        let mut state = AddUserState::default();
        state.name = "   ".to_string();
        state.handle_key(make_key(KeyCode::Enter));
        assert!(!state.is_confirmed());
    }

    #[test]
    fn test_set_success() {
        let mut state = AddUserState::default();
        state.confirmed = true;
        state.set_success("alice".to_string(), "uuid-123".to_string());
        assert!(!state.confirmed);
        match &state.result {
            AddUserResult::Success { name, uuid } => {
                assert_eq!(name, "alice");
                assert_eq!(uuid, "uuid-123");
            }
            _ => panic!("Expected Success"),
        }
    }

    #[test]
    fn test_set_error() {
        let mut state = AddUserState::default();
        state.confirmed = true;
        state.set_error("connection failed".to_string());
        assert!(!state.confirmed);
        match &state.result {
            AddUserResult::Error(msg) => assert_eq!(msg, "connection failed"),
            _ => panic!("Expected Error"),
        }
    }

    #[test]
    fn test_keys_not_consumed_when_showing_result() {
        let mut state = AddUserState::default();
        state.result = AddUserResult::Success {
            name: "alice".to_string(),
            uuid: "123".to_string(),
        };
        // Typing should not be consumed - let app handle navigation
        assert!(!state.handle_key(make_key(KeyCode::Char('x'))));
        assert_eq!(state.name, ""); // name unchanged
    }

    #[test]
    fn test_keys_not_consumed_when_showing_error() {
        let mut state = AddUserState::default();
        state.result = AddUserResult::Error("fail".to_string());
        assert!(!state.handle_key(make_key(KeyCode::Char('x'))));
    }

    #[test]
    fn test_centered_rect() {
        let area = Rect::new(0, 0, 100, 50);
        let rect = centered_rect(40, 10, area);
        assert_eq!(rect.x, 30);
        assert_eq!(rect.y, 20);
        assert_eq!(rect.width, 40);
        assert_eq!(rect.height, 10);
    }

    #[test]
    fn test_centered_rect_larger_than_area() {
        let area = Rect::new(0, 0, 20, 10);
        let rect = centered_rect(40, 20, area);
        assert_eq!(rect.width, 20); // clamped
        assert_eq!(rect.height, 10); // clamped
    }

    #[test]
    fn test_is_confirmed_requires_both() {
        let mut state = AddUserState::default();
        // Neither set
        assert!(!state.is_confirmed());

        // Confirmed but empty name
        state.confirmed = true;
        assert!(!state.is_confirmed());

        // Name but not confirmed
        state.confirmed = false;
        state.name = "alice".to_string();
        assert!(!state.is_confirmed());

        // Both set
        state.confirmed = true;
        assert!(state.is_confirmed());
    }
}
