use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Terminal;

use crate::config::Config;
use crate::ui::add_user::{self, AddUserResult, AddUserState};
use crate::ui::dashboard::{self, DashboardState};
use crate::ui::setup::{self, SetupState};
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
    pub setup_state: SetupState,
    pub dashboard_state: DashboardState,
    pub add_user_state: AddUserState,
    pub config: Config,
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
            setup_state: SetupState::default(),
            dashboard_state: DashboardState::default(),
            add_user_state: AddUserState::default(),
            config: Config::default(),
        }
    }

    pub fn with_config(config: Config) -> Self {
        let has_config = config.has_connection_info();
        let mut dashboard_state = DashboardState::default();
        if has_config {
            if let Some(ref host) = config.host {
                dashboard_state.server_host = host.clone();
            } else if let Some(ref ssh_host) = config.ssh_host {
                dashboard_state.server_host = ssh_host.clone();
            }
        }
        Self {
            screen: if has_config {
                Screen::Dashboard
            } else {
                Screen::Setup
            },
            running: true,
            last_refresh: Instant::now(),
            status_message: String::new(),
            setup_state: SetupState::from_config(&config),
            dashboard_state,
            add_user_state: AddUserState::default(),
            config,
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
            KeyCode::Char('j') | KeyCode::Down => {
                self.dashboard_state.select_next();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.dashboard_state.select_previous();
            }
            KeyCode::Enter => {
                if self.dashboard_state.selected_user().is_some() {
                    self.screen = Screen::UserDetail;
                }
            }
            KeyCode::Char('a') => {
                self.add_user_state.reset();
                self.screen = Screen::AddUser;
            }
            KeyCode::Char('d') => {
                if self.dashboard_state.selected_user().is_some() {
                    self.screen = Screen::UserDetail;
                    // Delete confirmation happens in user detail view
                }
            }
            _ => {}
        }
    }

    fn handle_setup_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.quit();
                return;
            }
            _ => {}
        }

        self.setup_state.handle_key(key);

        // Check if save was requested
        if self.setup_state.save_requested {
            self.setup_state.save_requested = false;
            let new_config = self.setup_state.to_config();
            if !new_config.has_connection_info() {
                self.setup_state.test_result =
                    setup::TestResult::Error("Please enter a host or SSH config alias".to_string());
                return;
            }
            match new_config.save() {
                Ok(()) => {
                    self.config = new_config;
                    self.screen = Screen::Dashboard;
                    self.status_message = "Config saved. Connected.".to_string();
                }
                Err(e) => {
                    self.setup_state.test_result =
                        setup::TestResult::Error(format!("Save failed: {}", e));
                }
            }
        }

        // Check if test was requested
        if self.setup_state.test_requested {
            self.setup_state.test_requested = false;
            if !self.setup_state.has_connection_info() {
                self.setup_state.test_result =
                    setup::TestResult::Error("Please enter a host or SSH config alias".to_string());
            } else {
                // For now, validate config structure (actual SSH test needs async)
                self.setup_state.test_result = setup::TestResult::Success(
                    "Config looks valid (SSH test requires running connection)".to_string(),
                );
            }
        }
    }

    fn handle_user_detail_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.screen = Screen::Dashboard,
            _ => {}
        }
    }

    fn handle_add_user_key(&mut self, key: KeyEvent) {
        // Let the add_user state handle input first
        if self.add_user_state.handle_key(key) {
            // Check if user confirmed the add
            if self.add_user_state.is_confirmed() {
                // For now, simulate success (actual SSH call happens in async context)
                // The async main loop will check is_confirmed() and call the API
                self.status_message = format!(
                    "Adding user '{}'...",
                    self.add_user_state.name
                );
            }
            return;
        }

        // Keys not consumed by add_user state
        match key.code {
            KeyCode::Esc => {
                self.add_user_state.reset();
                self.screen = Screen::Dashboard;
            }
            KeyCode::Char('q') => {
                // If showing success, offer QR view (will be wired in Task 13)
                if let AddUserResult::Success { .. } = &self.add_user_state.result {
                    self.screen = Screen::QrView;
                }
            }
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
    pub fn draw(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
        // Tick spinner when loading
        if self.dashboard_state.loading {
            self.dashboard_state.tick_spinner();
        }

        let screen = self.screen;
        let screen_label = self.screen_label();
        let keybinds = self.keybind_hints();
        let status_message = self.status_message.clone();

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

            Self::draw_header_static(frame, chunks[0], screen_label);

            match screen {
                Screen::Dashboard => dashboard::draw(&mut self.dashboard_state, frame, chunks[1]),
                Screen::Setup => setup::draw(&self.setup_state, frame, chunks[1]),
                Screen::AddUser => {
                    // Draw dashboard underneath, then overlay the dialog
                    dashboard::draw(&mut self.dashboard_state, frame, chunks[1]);
                    add_user::draw(&self.add_user_state, frame, chunks[1]);
                }
                _ => Self::draw_placeholder_widget(frame, chunks[1], screen_label),
            }

            Self::draw_status_bar_static(frame, chunks[2], &status_message, &keybinds);
        })?;
        Ok(())
    }

    fn draw_header_static(frame: &mut ratatui::Frame, area: Rect, screen_label: &str) {
        let header_block = Block::default()
            .borders(Borders::BOTTOM)
            .border_style(theme::border_style())
            .style(theme::header_style());

        let title_text = format!(
            " {} {} | {}",
            theme::APP_NAME,
            theme::APP_VERSION,
            screen_label
        );
        let header = Paragraph::new(Line::from(vec![Span::styled(
            title_text,
            theme::title_style(),
        )]))
        .block(header_block);

        frame.render_widget(header, area);
    }

    fn draw_placeholder_widget(frame: &mut ratatui::Frame, area: Rect, label: &str) {
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

    fn draw_status_bar_static(
        frame: &mut ratatui::Frame,
        area: Rect,
        status_message: &str,
        keybinds: &str,
    ) {
        let status = if status_message.is_empty() {
            keybinds.to_string()
        } else {
            format!("{} | {}", status_message, keybinds)
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

    #[test]
    fn test_with_config_has_connection() {
        let config = Config {
            host: Some("1.2.3.4".to_string()),
            ..Config::default()
        };
        let app = App::with_config(config);
        assert_eq!(app.screen, Screen::Dashboard);
        assert_eq!(app.setup_state.host, "1.2.3.4");
    }

    #[test]
    fn test_with_config_no_connection() {
        let config = Config::default();
        let app = App::with_config(config);
        assert_eq!(app.screen, Screen::Setup);
    }

    #[test]
    fn test_setup_tab_navigation() {
        let mut app = App::new(false);
        app.screen = Screen::Setup;
        assert_eq!(app.setup_state.focused, 0);
        app.handle_key(make_key(KeyCode::Tab));
        assert_eq!(app.setup_state.focused, 1);
        app.handle_key(make_key(KeyCode::Tab));
        assert_eq!(app.setup_state.focused, 2);
    }

    #[test]
    fn test_setup_typing() {
        let mut app = App::new(false);
        app.screen = Screen::Setup;
        // Focus is on SshHost (0)
        app.handle_key(make_key(KeyCode::Char('v')));
        app.handle_key(make_key(KeyCode::Char('p')));
        app.handle_key(make_key(KeyCode::Char('s')));
        assert_eq!(app.setup_state.ssh_host, "vps");
    }

    #[test]
    fn test_setup_save_without_connection_info_shows_error() {
        let mut app = App::new(false);
        app.screen = Screen::Setup;
        // Navigate to Save & Start button (index 7)
        app.setup_state.focused = 7;
        app.handle_key(make_key(KeyCode::Enter));
        // Should stay on Setup screen with error
        assert_eq!(app.screen, Screen::Setup);
        assert!(matches!(
            app.setup_state.test_result,
            setup::TestResult::Error(_)
        ));
    }

    #[test]
    fn test_setup_test_connection_without_info_shows_error() {
        let mut app = App::new(false);
        app.screen = Screen::Setup;
        // Navigate to Test Connection button (index 6)
        app.setup_state.focused = 6;
        app.handle_key(make_key(KeyCode::Enter));
        assert!(matches!(
            app.setup_state.test_result,
            setup::TestResult::Error(_)
        ));
    }

    #[test]
    fn test_setup_test_connection_with_info_shows_success() {
        let mut app = App::new(false);
        app.screen = Screen::Setup;
        app.setup_state.ssh_host = "vps-vpn".to_string();
        app.setup_state.focused = 6;
        app.handle_key(make_key(KeyCode::Enter));
        assert!(matches!(
            app.setup_state.test_result,
            setup::TestResult::Success(_)
        ));
    }

    // --- Dashboard navigation tests ---

    use crate::xray::types::{TrafficStats, XrayUser};

    fn make_test_user(name: &str) -> XrayUser {
        XrayUser {
            uuid: format!("{}-uuid-1234-5678-abcdefabcdef", name),
            name: name.to_string(),
            email: format!("{}@vpn", name),
            flow: "xtls-rprx-vision".to_string(),
            stats: TrafficStats::default(),
            online_count: 0,
        }
    }

    fn app_with_users(users: Vec<XrayUser>) -> App {
        let mut app = App::new(true);
        app.screen = Screen::Dashboard;
        app.dashboard_state.set_users(users);
        app
    }

    #[test]
    fn test_dashboard_j_selects_next() {
        let mut app = app_with_users(vec![make_test_user("alice"), make_test_user("bob")]);
        assert_eq!(app.dashboard_state.selected(), Some(0));
        app.handle_key(make_key(KeyCode::Char('j')));
        assert_eq!(app.dashboard_state.selected(), Some(1));
    }

    #[test]
    fn test_dashboard_k_selects_previous() {
        let mut app = app_with_users(vec![make_test_user("alice"), make_test_user("bob")]);
        app.dashboard_state.table_state.select(Some(1));
        app.handle_key(make_key(KeyCode::Char('k')));
        assert_eq!(app.dashboard_state.selected(), Some(0));
    }

    #[test]
    fn test_dashboard_down_selects_next() {
        let mut app = app_with_users(vec![make_test_user("alice"), make_test_user("bob")]);
        assert_eq!(app.dashboard_state.selected(), Some(0));
        app.handle_key(make_key(KeyCode::Down));
        assert_eq!(app.dashboard_state.selected(), Some(1));
    }

    #[test]
    fn test_dashboard_up_selects_previous() {
        let mut app = app_with_users(vec![make_test_user("alice"), make_test_user("bob")]);
        app.dashboard_state.table_state.select(Some(1));
        app.handle_key(make_key(KeyCode::Up));
        assert_eq!(app.dashboard_state.selected(), Some(0));
    }

    #[test]
    fn test_dashboard_enter_goes_to_user_detail() {
        let mut app = app_with_users(vec![make_test_user("alice")]);
        app.handle_key(make_key(KeyCode::Enter));
        assert_eq!(app.screen, Screen::UserDetail);
    }

    #[test]
    fn test_dashboard_enter_no_users_stays() {
        let mut app = App::new(true);
        app.screen = Screen::Dashboard;
        app.handle_key(make_key(KeyCode::Enter));
        assert_eq!(app.screen, Screen::Dashboard);
    }

    #[test]
    fn test_dashboard_a_goes_to_add_user() {
        let mut app = App::new(true);
        app.screen = Screen::Dashboard;
        app.handle_key(make_key(KeyCode::Char('a')));
        assert_eq!(app.screen, Screen::AddUser);
    }

    #[test]
    fn test_dashboard_d_with_user_goes_to_detail() {
        let mut app = app_with_users(vec![make_test_user("alice")]);
        app.handle_key(make_key(KeyCode::Char('d')));
        assert_eq!(app.screen, Screen::UserDetail);
    }

    #[test]
    fn test_dashboard_d_without_users_stays() {
        let mut app = App::new(true);
        app.screen = Screen::Dashboard;
        app.handle_key(make_key(KeyCode::Char('d')));
        assert_eq!(app.screen, Screen::Dashboard);
    }

    #[test]
    fn test_with_config_sets_dashboard_host() {
        let config = Config {
            host: Some("1.2.3.4".to_string()),
            ..Config::default()
        };
        let app = App::with_config(config);
        assert_eq!(app.dashboard_state.server_host, "1.2.3.4");
    }

    #[test]
    fn test_with_config_sets_dashboard_ssh_host() {
        let config = Config {
            ssh_host: Some("vps-vpn".to_string()),
            ..Config::default()
        };
        let app = App::with_config(config);
        assert_eq!(app.dashboard_state.server_host, "vps-vpn");
    }

    #[test]
    fn test_dashboard_navigation_wraps_with_j() {
        let mut app = app_with_users(vec![make_test_user("alice"), make_test_user("bob")]);
        app.dashboard_state.table_state.select(Some(1));
        app.handle_key(make_key(KeyCode::Char('j')));
        assert_eq!(app.dashboard_state.selected(), Some(0)); // wraps
    }

    #[test]
    fn test_dashboard_navigation_wraps_with_k() {
        let mut app = app_with_users(vec![make_test_user("alice"), make_test_user("bob")]);
        assert_eq!(app.dashboard_state.selected(), Some(0));
        app.handle_key(make_key(KeyCode::Char('k')));
        assert_eq!(app.dashboard_state.selected(), Some(1)); // wraps to end
    }

    #[test]
    fn test_dashboard_state_loading_default() {
        let app = App::new(true);
        assert!(app.dashboard_state.loading);
    }

    // --- Add user dialog integration tests ---

    #[test]
    fn test_add_user_typing_updates_state() {
        let mut app = App::new(true);
        app.screen = Screen::AddUser;
        app.handle_key(make_key(KeyCode::Char('b')));
        app.handle_key(make_key(KeyCode::Char('o')));
        app.handle_key(make_key(KeyCode::Char('b')));
        assert_eq!(app.add_user_state.name, "bob");
    }

    #[test]
    fn test_add_user_backspace() {
        let mut app = App::new(true);
        app.screen = Screen::AddUser;
        app.add_user_state.name = "alice".to_string();
        app.handle_key(make_key(KeyCode::Backspace));
        assert_eq!(app.add_user_state.name, "alic");
    }

    #[test]
    fn test_add_user_enter_confirms() {
        let mut app = App::new(true);
        app.screen = Screen::AddUser;
        app.add_user_state.name = "alice".to_string();
        app.handle_key(make_key(KeyCode::Enter));
        assert!(app.add_user_state.is_confirmed());
        assert!(app.status_message.contains("alice"));
    }

    #[test]
    fn test_add_user_enter_empty_does_not_confirm() {
        let mut app = App::new(true);
        app.screen = Screen::AddUser;
        app.handle_key(make_key(KeyCode::Enter));
        assert!(!app.add_user_state.is_confirmed());
    }

    #[test]
    fn test_add_user_esc_resets_and_returns() {
        let mut app = App::new(true);
        app.screen = Screen::AddUser;
        app.add_user_state.name = "alice".to_string();
        app.handle_key(make_key(KeyCode::Esc));
        assert_eq!(app.screen, Screen::Dashboard);
        assert_eq!(app.add_user_state.name, "");
    }

    #[test]
    fn test_add_user_dashboard_a_resets_state() {
        let mut app = App::new(true);
        app.screen = Screen::Dashboard;
        app.add_user_state.name = "leftover".to_string();
        app.handle_key(make_key(KeyCode::Char('a')));
        assert_eq!(app.screen, Screen::AddUser);
        assert_eq!(app.add_user_state.name, "");
    }

    #[test]
    fn test_add_user_success_q_goes_to_qr() {
        let mut app = App::new(true);
        app.screen = Screen::AddUser;
        app.add_user_state.set_success("alice".to_string(), "uuid-123".to_string());
        app.handle_key(make_key(KeyCode::Char('q')));
        assert_eq!(app.screen, Screen::QrView);
    }

    #[test]
    fn test_add_user_success_esc_returns_to_dashboard() {
        let mut app = App::new(true);
        app.screen = Screen::AddUser;
        app.add_user_state.set_success("alice".to_string(), "uuid-123".to_string());
        app.handle_key(make_key(KeyCode::Esc));
        assert_eq!(app.screen, Screen::Dashboard);
    }

    #[test]
    fn test_add_user_error_esc_returns_to_dashboard() {
        let mut app = App::new(true);
        app.screen = Screen::AddUser;
        app.add_user_state.set_error("connection failed".to_string());
        app.handle_key(make_key(KeyCode::Esc));
        assert_eq!(app.screen, Screen::Dashboard);
    }
}
