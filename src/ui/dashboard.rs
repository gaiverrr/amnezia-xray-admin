use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState};

use crate::ui::theme;
use crate::xray::types::XrayUser;

const SPINNER_FRAMES: &[char] = &[
    '\u{280b}', '\u{2819}', '\u{2839}', '\u{2838}', '\u{283c}', '\u{2834}', '\u{2826}', '\u{2827}',
    '\u{2807}', '\u{280f}',
];

/// Dashboard state holding user data and UI state
pub struct DashboardState {
    pub users: Vec<XrayUser>,
    pub table_state: TableState,
    pub server_version: String,
    pub server_host: String,
    pub total_upload: u64,
    pub total_download: u64,
    pub loading: bool,
    pub spinner_frame: usize,
}

impl Default for DashboardState {
    fn default() -> Self {
        Self {
            users: Vec::new(),
            table_state: TableState::default(),
            server_version: String::new(),
            server_host: String::new(),
            total_upload: 0,
            total_download: 0,
            loading: true,
            spinner_frame: 0,
        }
    }
}

impl DashboardState {
    /// Get the currently selected user
    pub fn selected_user(&self) -> Option<&XrayUser> {
        self.table_state.selected().and_then(|i| self.users.get(i))
    }

    /// Move selection down
    pub fn select_next(&mut self) {
        if self.users.is_empty() {
            return;
        }
        let next = match self.table_state.selected() {
            Some(i) => {
                if i >= self.users.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.table_state.select(Some(next));
    }

    /// Move selection up
    pub fn select_previous(&mut self) {
        if self.users.is_empty() {
            return;
        }
        let prev = match self.table_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.users.len() - 1
                } else {
                    i - 1
                }
            }
            None => self.users.len() - 1,
        };
        self.table_state.select(Some(prev));
    }

    /// Advance spinner animation
    pub fn tick_spinner(&mut self) {
        self.spinner_frame = (self.spinner_frame + 1) % SPINNER_FRAMES.len();
    }

    /// Get current spinner character
    pub fn spinner_char(&self) -> char {
        SPINNER_FRAMES[self.spinner_frame % SPINNER_FRAMES.len()]
    }

    /// Update user list and reset selection if needed
    pub fn set_users(&mut self, users: Vec<XrayUser>) {
        let selected = self.table_state.selected();
        self.users = users;
        if self.users.is_empty() {
            self.table_state.select(None);
        } else if let Some(i) = selected {
            if i >= self.users.len() {
                self.table_state.select(Some(self.users.len() - 1));
            }
        } else {
            self.table_state.select(Some(0));
        }
    }
}

/// Format bytes into human-readable string
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    const TB: u64 = 1024 * GB;

    if bytes >= TB {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Truncate UUID to first 8 characters
pub fn truncate_uuid(uuid: &str) -> &str {
    uuid.get(..8).unwrap_or(uuid)
}

/// Draw the dashboard view
pub fn draw(state: &mut DashboardState, frame: &mut ratatui::Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // server info header
            Constraint::Min(1),    // user table
        ])
        .split(area);

    draw_server_info(state, frame, chunks[0]);
    draw_user_table(state, frame, chunks[1]);
}

/// Draw server info header
fn draw_server_info(state: &DashboardState, frame: &mut ratatui::Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border_style())
        .title(Span::styled(" Server ", theme::secondary_style()))
        .style(theme::text_style());

    let loading_indicator = if state.loading {
        format!(" {} Loading...", state.spinner_char())
    } else {
        String::new()
    };

    let host_display = if state.server_host.is_empty() {
        "connecting...".to_string()
    } else {
        state.server_host.clone()
    };

    let version_display = if state.server_version.is_empty() {
        "-".to_string()
    } else {
        state.server_version.clone()
    };

    let info_line = Line::from(vec![
        Span::styled("  Host: ", theme::muted_style()),
        Span::styled(&host_display, theme::text_style()),
        Span::styled("  |  Xray: ", theme::muted_style()),
        Span::styled(&version_display, theme::text_style()),
        Span::styled("  |  ", theme::muted_style()),
        Span::styled(
            format!(
                "Up: {}  Down: {}",
                format_bytes(state.total_upload),
                format_bytes(state.total_download)
            ),
            theme::secondary_style(),
        ),
        Span::styled(
            format!("  |  Users: {}", state.users.len()),
            theme::muted_style(),
        ),
        Span::styled(loading_indicator, theme::accent_style()),
    ]);

    let content = Paragraph::new(vec![Line::from(""), info_line]).block(block);
    frame.render_widget(content, area);
}

/// Draw the user table
fn draw_user_table(state: &mut DashboardState, frame: &mut ratatui::Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border_style())
        .title(Span::styled(" Users ", theme::secondary_style()))
        .style(theme::text_style());

    if state.users.is_empty() {
        let msg = if state.loading {
            "  Loading user data..."
        } else {
            "  No users found. Press [a] to add a user."
        };
        let content = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(msg, theme::muted_style())),
        ])
        .block(block);
        frame.render_widget(content, area);
        return;
    }

    let header_cells = ["", "Name", "UUID", "Upload", "Download", "Online"]
        .iter()
        .map(|h| Cell::from(*h).style(theme::header_style()));
    let header = Row::new(header_cells).height(1);

    let rows: Vec<Row> = state
        .users
        .iter()
        .map(|user| {
            let online_text = if user.online_count > 0 {
                format!("{} ({})", "\u{25cf}", user.online_count)
            } else {
                "\u{25cf}".to_string()
            };

            let online_style = if user.online_count > 0 {
                theme::title_style()
            } else {
                theme::alert_style()
            };

            Row::new(vec![
                Cell::from(" "),
                Cell::from(Span::styled(user.name.clone(), theme::text_style())),
                Cell::from(Span::styled(
                    truncate_uuid(&user.uuid).to_string(),
                    theme::muted_style(),
                )),
                Cell::from(Span::styled(
                    format_bytes(user.stats.uplink),
                    theme::secondary_style(),
                )),
                Cell::from(Span::styled(
                    format_bytes(user.stats.downlink),
                    theme::secondary_style(),
                )),
                Cell::from(Span::styled(online_text, online_style)),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Min(12),
        Constraint::Length(10),
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Length(10),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .row_highlight_style(theme::selected_style())
        .highlight_symbol(">> ");

    frame.render_stateful_widget(table, area, &mut state.table_state);
}

#[cfg(test)]
impl DashboardState {
    pub fn selected(&self) -> Option<usize> {
        self.table_state.selected()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xray::types::TrafficStats;

    fn make_user(name: &str, uuid: &str, up: u64, down: u64, online: u32) -> XrayUser {
        XrayUser {
            uuid: uuid.to_string(),
            name: name.to_string(),
            email: format!("{}@vpn", name),
            flow: "xtls-rprx-vision".to_string(),
            stats: TrafficStats {
                uplink: up,
                downlink: down,
            },
            online_count: online,
        }
    }

    #[test]
    fn test_format_bytes_zero() {
        assert_eq!(format_bytes(0), "0 B");
    }

    #[test]
    fn test_format_bytes_bytes() {
        assert_eq!(format_bytes(512), "512 B");
    }

    #[test]
    fn test_format_bytes_kb() {
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
    }

    #[test]
    fn test_format_bytes_mb() {
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(5 * 1024 * 1024 + 512 * 1024), "5.5 MB");
    }

    #[test]
    fn test_format_bytes_gb() {
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0 GB");
        assert_eq!(
            format_bytes(2 * 1024 * 1024 * 1024 + 512 * 1024 * 1024),
            "2.5 GB"
        );
    }

    #[test]
    fn test_format_bytes_tb() {
        assert_eq!(format_bytes(1024u64 * 1024 * 1024 * 1024), "1.0 TB");
    }

    #[test]
    fn test_truncate_uuid_long() {
        assert_eq!(
            truncate_uuid("aaaaaaaa-1111-2222-3333-bbbbbbbbbbbb"),
            "aaaaaaaa"
        );
    }

    #[test]
    fn test_truncate_uuid_short() {
        assert_eq!(truncate_uuid("short"), "short");
    }

    #[test]
    fn test_truncate_uuid_exactly_8() {
        assert_eq!(truncate_uuid("12345678"), "12345678");
    }

    #[test]
    fn test_dashboard_state_default() {
        let state = DashboardState::default();
        assert!(state.users.is_empty());
        assert!(state.loading);
        assert_eq!(state.spinner_frame, 0);
        assert!(state.selected().is_none());
    }

    #[test]
    fn test_select_next_empty() {
        let mut state = DashboardState::default();
        state.select_next();
        assert!(state.selected().is_none());
    }

    #[test]
    fn test_select_previous_empty() {
        let mut state = DashboardState::default();
        state.select_previous();
        assert!(state.selected().is_none());
    }

    #[test]
    fn test_select_next_wraps() {
        let mut state = DashboardState::default();
        state.set_users(vec![
            make_user("alice", "aaa", 0, 0, 0),
            make_user("bob", "bbb", 0, 0, 0),
            make_user("charlie", "ccc", 0, 0, 0),
        ]);
        assert_eq!(state.selected(), Some(0));
        state.select_next();
        assert_eq!(state.selected(), Some(1));
        state.select_next();
        assert_eq!(state.selected(), Some(2));
        state.select_next();
        assert_eq!(state.selected(), Some(0)); // wraps
    }

    #[test]
    fn test_select_previous_wraps() {
        let mut state = DashboardState::default();
        state.set_users(vec![
            make_user("alice", "aaa", 0, 0, 0),
            make_user("bob", "bbb", 0, 0, 0),
        ]);
        assert_eq!(state.selected(), Some(0));
        state.select_previous();
        assert_eq!(state.selected(), Some(1)); // wraps to end
        state.select_previous();
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn test_select_next_from_none() {
        let mut state = DashboardState::default();
        state.users = vec![make_user("alice", "aaa", 0, 0, 0)];
        // table_state has no selection
        state.select_next();
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn test_select_previous_from_none() {
        let mut state = DashboardState::default();
        state.users = vec![
            make_user("alice", "aaa", 0, 0, 0),
            make_user("bob", "bbb", 0, 0, 0),
        ];
        state.select_previous();
        assert_eq!(state.selected(), Some(1)); // selects last
    }

    #[test]
    fn test_set_users_selects_first() {
        let mut state = DashboardState::default();
        assert!(state.selected().is_none());
        state.set_users(vec![make_user("alice", "aaa", 0, 0, 0)]);
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn test_set_users_empty_clears_selection() {
        let mut state = DashboardState::default();
        state.set_users(vec![make_user("alice", "aaa", 0, 0, 0)]);
        assert_eq!(state.selected(), Some(0));
        state.set_users(vec![]);
        assert!(state.selected().is_none());
    }

    #[test]
    fn test_set_users_preserves_selection() {
        let mut state = DashboardState::default();
        state.set_users(vec![
            make_user("alice", "aaa", 0, 0, 0),
            make_user("bob", "bbb", 0, 0, 0),
            make_user("charlie", "ccc", 0, 0, 0),
        ]);
        state.table_state.select(Some(1));
        // Update with same-size list
        state.set_users(vec![
            make_user("alice", "aaa", 100, 200, 1),
            make_user("bob", "bbb", 300, 400, 0),
            make_user("charlie", "ccc", 0, 0, 0),
        ]);
        assert_eq!(state.selected(), Some(1)); // preserved
    }

    #[test]
    fn test_set_users_clamps_selection() {
        let mut state = DashboardState::default();
        state.set_users(vec![
            make_user("alice", "aaa", 0, 0, 0),
            make_user("bob", "bbb", 0, 0, 0),
            make_user("charlie", "ccc", 0, 0, 0),
        ]);
        state.table_state.select(Some(2)); // select last
                                           // Shrink list
        state.set_users(vec![make_user("alice", "aaa", 0, 0, 0)]);
        assert_eq!(state.selected(), Some(0)); // clamped
    }

    #[test]
    fn test_selected_user() {
        let mut state = DashboardState::default();
        state.set_users(vec![
            make_user("alice", "aaa", 0, 0, 0),
            make_user("bob", "bbb", 0, 0, 0),
        ]);
        state.table_state.select(Some(1));
        let user = state.selected_user().unwrap();
        assert_eq!(user.name, "bob");
    }

    #[test]
    fn test_selected_user_none() {
        let state = DashboardState::default();
        assert!(state.selected_user().is_none());
    }

    #[test]
    fn test_tick_spinner() {
        let mut state = DashboardState::default();
        assert_eq!(state.spinner_frame, 0);
        state.tick_spinner();
        assert_eq!(state.spinner_frame, 1);
        // Advance through all frames
        for _ in 0..SPINNER_FRAMES.len() - 1 {
            state.tick_spinner();
        }
        assert_eq!(state.spinner_frame, 0); // wrapped
    }

    #[test]
    fn test_spinner_char() {
        let state = DashboardState::default();
        assert_eq!(state.spinner_char(), SPINNER_FRAMES[0]);
    }
}
