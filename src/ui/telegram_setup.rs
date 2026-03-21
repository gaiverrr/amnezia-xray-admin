use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::theme;

/// Fields in the Telegram bot setup form
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelegramField {
    Token,
    DeployToVps,
}

impl TelegramField {
    pub const ALL: [TelegramField; 2] = [TelegramField::Token, TelegramField::DeployToVps];

    #[allow(dead_code)]
    pub fn is_button(&self) -> bool {
        matches!(self, TelegramField::DeployToVps)
    }
}

/// Deployment progress steps
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum DeployStatus {
    None,
    Connecting,
    BuildingImage,
    CreatingCompose,
    StartingBot,
    Verifying,
    Success(String),
    Error(String),
}

/// State for the Telegram bot setup screen
#[derive(Debug, Clone)]
pub struct TelegramSetupState {
    pub token: String,
    pub focused: usize,
    pub deploy_status: DeployStatus,
    pub deploy_requested: bool,
}

impl Default for TelegramSetupState {
    fn default() -> Self {
        Self {
            token: String::new(),
            focused: 0,
            deploy_status: DeployStatus::None,
            deploy_requested: false,
        }
    }
}

impl TelegramSetupState {
    /// Pre-fill token from config if available
    pub fn from_token(token: Option<&str>) -> Self {
        Self {
            token: token.unwrap_or_default().to_string(),
            ..Default::default()
        }
    }

    /// Get the currently focused field
    pub fn focused_field(&self) -> TelegramField {
        TelegramField::ALL[self.focused]
    }

    /// Move focus to the next field
    pub fn focus_next(&mut self) {
        self.focused = (self.focused + 1) % TelegramField::ALL.len();
    }

    /// Move focus to the previous field
    pub fn focus_prev(&mut self) {
        if self.focused == 0 {
            self.focused = TelegramField::ALL.len() - 1;
        } else {
            self.focused -= 1;
        }
    }

    /// Handle a key event, return true if the event was consumed
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Tab | KeyCode::Down => {
                self.focus_next();
                true
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.focus_prev();
                true
            }
            KeyCode::Enter => {
                if self.focused_field() == TelegramField::DeployToVps {
                    if !self.token.trim().is_empty() {
                        self.deploy_requested = true;
                    }
                } else {
                    self.focus_next();
                }
                true
            }
            KeyCode::Char(c) => {
                if self.focused_field() == TelegramField::Token {
                    self.token.push(c);
                    true
                } else {
                    false
                }
            }
            KeyCode::Backspace => {
                if self.focused_field() == TelegramField::Token {
                    self.token.pop();
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Reset the state
    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.deploy_status = DeployStatus::None;
        self.deploy_requested = false;
    }
}

/// Validate a Telegram bot token format (roughly: digits:alphanumeric-underscore-dash)
pub fn is_valid_token(token: &str) -> bool {
    let token = token.trim();
    if token.is_empty() {
        return false;
    }
    // Format: <bot_id>:<secret> where bot_id is digits, secret is alphanumeric with - and _
    let parts: Vec<&str> = token.splitn(2, ':').collect();
    if parts.len() != 2 {
        return false;
    }
    let bot_id = parts[0];
    let secret = parts[1];
    !bot_id.is_empty()
        && bot_id.chars().all(|c| c.is_ascii_digit())
        && !secret.is_empty()
        && secret
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Generate a docker-compose.yml content for the Telegram bot
pub fn generate_compose_yaml(token: &str, container: &str) -> String {
    format!(
        r#"services:
  axadmin-bot:
    image: axadmin:latest
    build: .
    restart: unless-stopped
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
      - axadmin-data:/root/.config/amnezia-xray-admin
    environment:
      - TELEGRAM_TOKEN={}
    command: --telegram-bot --local --container {}
volumes:
  axadmin-data:
"#,
        token, container
    )
}

/// Draw the Telegram bot setup screen
pub fn draw(state: &TelegramSetupState, frame: &mut ratatui::Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border_style())
        .title(Span::styled(" Telegram Bot Setup ", theme::accent_style()))
        .style(theme::text_style());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6), // instructions
            Constraint::Length(1), // spacer
            Constraint::Length(1), // token input
            Constraint::Length(1), // spacer
            Constraint::Length(1), // deploy button
            Constraint::Length(1), // spacer
            Constraint::Length(3), // deploy status
            Constraint::Min(0),    // padding
        ])
        .split(inner);

    // Instructions
    let instructions = vec![
        Line::from(Span::styled(
            "  Setup your Telegram bot to manage VPN users remotely:",
            theme::secondary_style(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  1. Open @BotFather in Telegram",
            theme::text_style(),
        )),
        Line::from(Span::styled(
            "  2. Send /newbot and follow the prompts",
            theme::text_style(),
        )),
        Line::from(Span::styled(
            "  3. Copy the bot token and paste it below",
            theme::text_style(),
        )),
        Line::from(Span::styled(
            "  4. After deploy, send /start to your bot to become admin",
            theme::muted_style(),
        )),
    ];
    frame.render_widget(Paragraph::new(instructions), chunks[0]);

    // Token input
    draw_token_input(state, frame, chunks[2]);

    // Deploy button
    draw_deploy_button(state, frame, chunks[4]);

    // Deploy status
    draw_deploy_status(state, frame, chunks[6]);
}

fn draw_token_input(state: &TelegramSetupState, frame: &mut ratatui::Frame, area: Rect) {
    let focused = state.focused_field() == TelegramField::Token;

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(2),  // margin
            Constraint::Length(14), // label
            Constraint::Length(2),  // separator
            Constraint::Min(20),    // input
        ])
        .split(area);

    let label_style = if focused {
        theme::secondary_style()
    } else {
        theme::muted_style()
    };

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled("Bot Token", label_style))),
        cols[1],
    );

    let separator = if focused { "> " } else { ": " };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(separator, label_style))),
        cols[2],
    );

    // Mask token for display (show first 5 chars + asterisks)
    let display_value = if state.token.is_empty() {
        if focused {
            "_".to_string()
        } else {
            String::new()
        }
    } else {
        let masked = mask_token(&state.token);
        if focused {
            format!("{}_", masked)
        } else {
            masked
        }
    };

    let value_style = if focused {
        theme::title_style()
    } else {
        theme::text_style()
    };

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(display_value, value_style))),
        cols[3],
    );
}

fn draw_deploy_button(state: &TelegramSetupState, frame: &mut ratatui::Frame, area: Rect) {
    let focused = state.focused_field() == TelegramField::DeployToVps;

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(2), Constraint::Min(20)])
        .split(area);

    let style = if focused {
        theme::selected_style()
    } else {
        theme::secondary_style()
    };

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled("[ Deploy to VPS ]", style))),
        cols[1],
    );
}

fn draw_deploy_status(state: &TelegramSetupState, frame: &mut ratatui::Frame, area: Rect) {
    let (text, style) = match &state.deploy_status {
        DeployStatus::None => return,
        DeployStatus::Connecting => (
            "  Connecting to VPS...".to_string(),
            theme::secondary_style(),
        ),
        DeployStatus::BuildingImage => (
            "  Building Docker image (this may take a few minutes)...".to_string(),
            theme::secondary_style(),
        ),
        DeployStatus::CreatingCompose => (
            "  Creating docker-compose configuration...".to_string(),
            theme::secondary_style(),
        ),
        DeployStatus::StartingBot => (
            "  Starting bot container...".to_string(),
            theme::secondary_style(),
        ),
        DeployStatus::Verifying => (
            "  Verifying bot is running...".to_string(),
            theme::secondary_style(),
        ),
        DeployStatus::Success(msg) => (format!("  OK: {}", msg), theme::title_style()),
        DeployStatus::Error(msg) => (format!("  Error: {}", msg), theme::alert_style()),
    };

    frame.render_widget(Paragraph::new(Line::from(Span::styled(text, style))), area);
}

/// Mask a token for display: show first 5 chars, then asterisks
pub fn mask_token(token: &str) -> String {
    if token.len() <= 5 {
        token.to_string()
    } else {
        let visible: String = token.chars().take(5).collect();
        let hidden_len = token.len() - 5;
        format!("{}{}", visible, "*".repeat(hidden_len.min(30)))
    }
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
        let state = TelegramSetupState::default();
        assert_eq!(state.token, "");
        assert_eq!(state.focused, 0);
        assert!(!state.deploy_requested);
        assert_eq!(state.deploy_status, DeployStatus::None);
    }

    #[test]
    fn test_from_token() {
        let state = TelegramSetupState::from_token(Some("123:abc"));
        assert_eq!(state.token, "123:abc");
    }

    #[test]
    fn test_from_token_none() {
        let state = TelegramSetupState::from_token(None);
        assert_eq!(state.token, "");
    }

    #[test]
    fn test_focus_next() {
        let mut state = TelegramSetupState::default();
        assert_eq!(state.focused_field(), TelegramField::Token);
        state.focus_next();
        assert_eq!(state.focused_field(), TelegramField::DeployToVps);
        state.focus_next();
        assert_eq!(state.focused_field(), TelegramField::Token); // wraps
    }

    #[test]
    fn test_focus_prev() {
        let mut state = TelegramSetupState::default();
        state.focus_prev();
        assert_eq!(state.focused_field(), TelegramField::DeployToVps); // wraps
    }

    #[test]
    fn test_typing_token() {
        let mut state = TelegramSetupState::default();
        state.handle_key(make_key(KeyCode::Char('1')));
        state.handle_key(make_key(KeyCode::Char('2')));
        state.handle_key(make_key(KeyCode::Char('3')));
        assert_eq!(state.token, "123");
    }

    #[test]
    fn test_backspace_token() {
        let mut state = TelegramSetupState::default();
        state.token = "123".to_string();
        state.handle_key(make_key(KeyCode::Backspace));
        assert_eq!(state.token, "12");
    }

    #[test]
    fn test_typing_on_button_not_consumed() {
        let mut state = TelegramSetupState::default();
        state.focused = 1; // DeployToVps
        let consumed = state.handle_key(make_key(KeyCode::Char('x')));
        assert!(!consumed);
    }

    #[test]
    fn test_enter_on_token_moves_to_deploy() {
        let mut state = TelegramSetupState::default();
        state.handle_key(make_key(KeyCode::Enter));
        assert_eq!(state.focused_field(), TelegramField::DeployToVps);
    }

    #[test]
    fn test_enter_on_deploy_with_token() {
        let mut state = TelegramSetupState::default();
        state.token = "123:abc".to_string();
        state.focused = 1;
        state.handle_key(make_key(KeyCode::Enter));
        assert!(state.deploy_requested);
    }

    #[test]
    fn test_enter_on_deploy_without_token() {
        let mut state = TelegramSetupState::default();
        state.focused = 1;
        state.handle_key(make_key(KeyCode::Enter));
        assert!(!state.deploy_requested);
    }

    #[test]
    fn test_tab_moves_focus() {
        let mut state = TelegramSetupState::default();
        state.handle_key(make_key(KeyCode::Tab));
        assert_eq!(state.focused, 1);
    }

    #[test]
    fn test_backtab_moves_focus() {
        let mut state = TelegramSetupState::default();
        state.focused = 1;
        state.handle_key(make_key(KeyCode::BackTab));
        assert_eq!(state.focused, 0);
    }

    #[test]
    fn test_reset() {
        let mut state = TelegramSetupState::default();
        state.deploy_status = DeployStatus::Success("done".to_string());
        state.deploy_requested = true;
        state.reset();
        assert_eq!(state.deploy_status, DeployStatus::None);
        assert!(!state.deploy_requested);
    }

    #[test]
    fn test_is_valid_token_valid() {
        assert!(is_valid_token("123456789:ABCdefGHI-jklMNO_pqrSTU"));
        assert!(is_valid_token("1:a"));
    }

    #[test]
    fn test_is_valid_token_invalid() {
        assert!(!is_valid_token(""));
        assert!(!is_valid_token("no-colon"));
        assert!(!is_valid_token(":missing-id"));
        assert!(!is_valid_token("abc:secret")); // bot ID must be digits
        assert!(!is_valid_token("123:")); // empty secret
        assert!(!is_valid_token("123:sec ret")); // space in secret
    }

    #[test]
    fn test_mask_token_short() {
        assert_eq!(mask_token("123"), "123");
        assert_eq!(mask_token("12345"), "12345");
    }

    #[test]
    fn test_mask_token_long() {
        let masked = mask_token("123456789:ABCdef");
        assert!(masked.starts_with("12345"));
        assert!(masked.contains('*'));
        assert!(!masked.contains("6789"));
    }

    #[test]
    fn test_generate_compose_yaml() {
        let yaml = generate_compose_yaml("123:abc", "amnezia-xray");
        assert!(yaml.contains("TELEGRAM_TOKEN=123:abc"));
        assert!(yaml.contains("--container amnezia-xray"));
        assert!(yaml.contains("docker.sock"));
    }

    #[test]
    fn test_field_is_button() {
        assert!(!TelegramField::Token.is_button());
        assert!(TelegramField::DeployToVps.is_button());
    }
}
