use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Terminal;

use crate::ui::theme;

/// Screens in the application
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Setup,
    Dashboard,
    UserDetail,
    AddUser,
    QrView,
}

/// Refresh interval for stats polling
const REFRESH_INTERVAL: Duration = Duration::from_secs(5);

/// Core application state
pub struct App {
    pub screen: Screen,
    pub running: bool,
    pub last_refresh: Instant,
    pub status_message: String,
}

impl App {
    pub fn new(has_config: bool) -> Self {
        Self {
            screen: if has_config {
                Screen::Dashboard
            } else {
                Screen::Setup
            },
            running: true,
            last_refresh: Instant::now(),
            status_message: String::new(),
        }
    }

    pub fn quit(&mut self) {
        self.running = false;
    }

    /// Handle a key event and update state accordingly
    pub fn handle_key(&mut self, key: KeyEvent) {
        // Global: Ctrl+C or Ctrl+Q always quits
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && (key.code == KeyCode::Char('c') || key.code == KeyCode::Char('q'))
        {
            self.quit();
            return;
        }

        match self.screen {
            Screen::Dashboard => self.handle_dashboard_key(key),
            Screen::Setup => self.handle_setup_key(key),
            Screen::UserDetail => self.handle_user_detail_key(key),
            Screen::AddUser => self.handle_add_user_key(key),
            Screen::QrView => self.handle_qr_view_key(key),
        }
    }

    fn handle_dashboard_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.quit(),
            KeyCode::Char('r') => {
                self.last_refresh = Instant::now();
                self.status_message = "Refreshing...".to_string();
            }
            _ => {}
        }
    }

    fn handle_setup_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.quit(),
            _ => {}
        }
    }

    fn handle_user_detail_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.screen = Screen::Dashboard,
            _ => {}
        }
    }

    fn handle_add_user_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.screen = Screen::Dashboard,
            _ => {}
        }
    }

    fn handle_qr_view_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.screen = Screen::Dashboard,
            _ => {}
        }
    }

    /// Check if a periodic refresh is due
    pub fn should_refresh(&self) -> bool {
        self.last_refresh.elapsed() >= REFRESH_INTERVAL
    }

    /// Mark refresh as done
    pub fn mark_refreshed(&mut self) {
        self.last_refresh = Instant::now();
    }

    /// Draw the UI
    pub fn draw(&self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
        terminal.draw(|frame| {
            let size = frame.area();

            // Main layout: header (3 lines), body (flex), status bar (1 line)
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3), // header
                    Constraint::Min(1),    // body
                    Constraint::Length(1), // status bar
                ])
                .split(size);

            self.draw_header(frame, chunks[0]);
            self.draw_body(frame, chunks[1]);
            self.draw_status_bar(frame, chunks[2]);
        })?;
        Ok(())
    }

    fn draw_header(&self, frame: &mut ratatui::Frame, area: Rect) {
        let header_block = Block::default()
            .borders(Borders::BOTTOM)
            .border_style(theme::border_style())
            .style(theme::header_style());

        let title_text = format!(
            " {} {} | {}",
            theme::APP_NAME,
            theme::APP_VERSION,
            self.screen_label()
        );
        let header = Paragraph::new(Line::from(vec![Span::styled(
            title_text,
            theme::title_style(),
        )]))
        .block(header_block);

        frame.render_widget(header, area);
    }

    fn draw_body(&self, frame: &mut ratatui::Frame, area: Rect) {
        match self.screen {
            Screen::Dashboard => self.draw_dashboard_placeholder(frame, area),
            Screen::Setup => self.draw_setup_placeholder(frame, area),
            Screen::UserDetail => self.draw_placeholder(frame, area, "User Detail"),
            Screen::AddUser => self.draw_placeholder(frame, area, "Add User"),
            Screen::QrView => self.draw_placeholder(frame, area, "QR Code"),
        }
    }

    fn draw_dashboard_placeholder(&self, frame: &mut ratatui::Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme::border_style())
            .title(Span::styled(" Dashboard ", theme::secondary_style()))
            .style(theme::text_style());

        let logo_lines: Vec<Line> = theme::LOGO
            .lines()
            .map(|l| Line::from(Span::styled(l, theme::title_style())))
            .collect();

        let mut lines = logo_lines;
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Connecting to server...",
            theme::muted_style(),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  No user data loaded yet.",
            theme::muted_style(),
        )));

        let content = Paragraph::new(lines).block(block);
        frame.render_widget(content, area);
    }

    fn draw_setup_placeholder(&self, frame: &mut ratatui::Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme::border_style())
            .title(Span::styled(" First-Run Setup ", theme::accent_style()))
            .style(theme::text_style());

        let mut lines: Vec<Line> = theme::LOGO
            .lines()
            .map(|l| Line::from(Span::styled(l, theme::title_style())))
            .collect();
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Welcome! Configure your connection to get started.",
            theme::secondary_style(),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Setup wizard will be implemented in the next task.",
            theme::muted_style(),
        )));

        let content = Paragraph::new(lines).block(block);
        frame.render_widget(content, area);
    }

    fn draw_placeholder(&self, frame: &mut ratatui::Frame, area: Rect, label: &str) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme::border_style())
            .title(Span::styled(
                format!(" {} ", label),
                theme::secondary_style(),
            ))
            .style(theme::text_style());

        let content = Paragraph::new(Line::from(Span::styled(
            format!("  {} view - coming soon", label),
            theme::muted_style(),
        )))
        .block(block);
        frame.render_widget(content, area);
    }

    fn draw_status_bar(&self, frame: &mut ratatui::Frame, area: Rect) {
        let keybinds = self.keybind_hints();
        let status = if self.status_message.is_empty() {
            keybinds
        } else {
            format!("{} | {}", self.status_message, keybinds)
        };

        let bar = Paragraph::new(Line::from(Span::styled(status, theme::status_style())));
        frame.render_widget(bar, area);
    }

    fn screen_label(&self) -> &'static str {
        match self.screen {
            Screen::Setup => "Setup",
            Screen::Dashboard => "Dashboard",
            Screen::UserDetail => "User Detail",
            Screen::AddUser => "Add User",
            Screen::QrView => "QR Code",
        }
    }

    fn keybind_hints(&self) -> String {
        match self.screen {
            Screen::Dashboard => {
                "[a]dd user  [d]elete  [r]efresh  [q]uit  [Enter] detail".to_string()
            }
            Screen::Setup => "[Tab] next field  [Enter] confirm  [Esc] quit".to_string(),
            Screen::UserDetail => "[q] back  [d]elete  [c]opy URL  [Q]R code".to_string(),
            Screen::AddUser => "[Enter] confirm  [Esc] cancel".to_string(),
            Screen::QrView => "[Esc/q] back".to_string(),
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new(false)
    }
}

/// Initialize the terminal for TUI rendering
pub fn init_terminal() -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend)
}

/// Restore terminal to normal state
pub fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

/// Run the main event loop
pub fn run(app: &mut App, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    while app.running {
        app.draw(terminal)?;

        // Poll for events with a timeout so we can do periodic refresh
        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                app.handle_key(key);
            }
        }

        // Periodic refresh check
        if app.should_refresh() {
            app.mark_refreshed();
            // Future: trigger data reload here
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn make_key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn make_ctrl_key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn test_new_with_config() {
        let app = App::new(true);
        assert_eq!(app.screen, Screen::Dashboard);
        assert!(app.running);
    }

    #[test]
    fn test_new_without_config() {
        let app = App::new(false);
        assert_eq!(app.screen, Screen::Setup);
        assert!(app.running);
    }

    #[test]
    fn test_default_starts_setup() {
        let app = App::default();
        assert_eq!(app.screen, Screen::Setup);
    }

    #[test]
    fn test_quit() {
        let mut app = App::new(true);
        assert!(app.running);
        app.quit();
        assert!(!app.running);
    }

    #[test]
    fn test_ctrl_c_quits() {
        let mut app = App::new(true);
        app.handle_key(make_ctrl_key(KeyCode::Char('c')));
        assert!(!app.running);
    }

    #[test]
    fn test_ctrl_q_quits() {
        let mut app = App::new(true);
        app.handle_key(make_ctrl_key(KeyCode::Char('q')));
        assert!(!app.running);
    }

    #[test]
    fn test_dashboard_q_quits() {
        let mut app = App::new(true);
        app.screen = Screen::Dashboard;
        app.handle_key(make_key(KeyCode::Char('q')));
        assert!(!app.running);
    }

    #[test]
    fn test_dashboard_r_refreshes() {
        let mut app = App::new(true);
        app.screen = Screen::Dashboard;
        // Advance time artificially
        app.last_refresh = Instant::now() - Duration::from_secs(10);
        app.handle_key(make_key(KeyCode::Char('r')));
        assert_eq!(app.status_message, "Refreshing...");
        assert!(app.last_refresh.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn test_user_detail_esc_returns_to_dashboard() {
        let mut app = App::new(true);
        app.screen = Screen::UserDetail;
        app.handle_key(make_key(KeyCode::Esc));
        assert_eq!(app.screen, Screen::Dashboard);
    }

    #[test]
    fn test_user_detail_q_returns_to_dashboard() {
        let mut app = App::new(true);
        app.screen = Screen::UserDetail;
        app.handle_key(make_key(KeyCode::Char('q')));
        assert_eq!(app.screen, Screen::Dashboard);
    }

    #[test]
    fn test_add_user_esc_returns_to_dashboard() {
        let mut app = App::new(true);
        app.screen = Screen::AddUser;
        app.handle_key(make_key(KeyCode::Esc));
        assert_eq!(app.screen, Screen::Dashboard);
    }

    #[test]
    fn test_qr_view_esc_returns_to_dashboard() {
        let mut app = App::new(true);
        app.screen = Screen::QrView;
        app.handle_key(make_key(KeyCode::Esc));
        assert_eq!(app.screen, Screen::Dashboard);
    }

    #[test]
    fn test_qr_view_q_returns_to_dashboard() {
        let mut app = App::new(true);
        app.screen = Screen::QrView;
        app.handle_key(make_key(KeyCode::Char('q')));
        assert_eq!(app.screen, Screen::Dashboard);
    }

    #[test]
    fn test_setup_esc_quits() {
        let mut app = App::new(false);
        app.screen = Screen::Setup;
        app.handle_key(make_key(KeyCode::Esc));
        assert!(!app.running);
    }

    #[test]
    fn test_should_refresh() {
        let mut app = App::new(true);
        app.last_refresh = Instant::now() - Duration::from_secs(10);
        assert!(app.should_refresh());
        app.mark_refreshed();
        assert!(!app.should_refresh());
    }

    #[test]
    fn test_screen_labels() {
        let app = App::new(true);
        assert_eq!(app.screen_label(), "Dashboard");

        let mut app2 = App::new(false);
        app2.screen = Screen::Setup;
        assert_eq!(app2.screen_label(), "Setup");

        app2.screen = Screen::UserDetail;
        assert_eq!(app2.screen_label(), "User Detail");

        app2.screen = Screen::AddUser;
        assert_eq!(app2.screen_label(), "Add User");

        app2.screen = Screen::QrView;
        assert_eq!(app2.screen_label(), "QR Code");
    }

    #[test]
    fn test_keybind_hints_nonempty() {
        let mut app = App::new(true);
        for screen in [
            Screen::Dashboard,
            Screen::Setup,
            Screen::UserDetail,
            Screen::AddUser,
            Screen::QrView,
        ] {
            app.screen = screen;
            assert!(
                !app.keybind_hints().is_empty(),
                "hints empty for {:?}",
                screen
            );
        }
    }

    #[test]
    fn test_unhandled_key_does_nothing() {
        let mut app = App::new(true);
        app.screen = Screen::Dashboard;
        let before_screen = app.screen;
        let before_running = app.running;
        app.handle_key(make_key(KeyCode::Char('z')));
        assert_eq!(app.screen, before_screen);
        assert_eq!(app.running, before_running);
    }
}
