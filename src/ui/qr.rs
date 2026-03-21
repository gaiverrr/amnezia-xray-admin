/// QR code rendering as unicode block characters for terminal display,
/// and as PNG images for Telegram bot.
///
/// Uses the `qrcode` crate to generate QR codes and renders them
/// using unicode half-block characters (U+2580 UPPER HALF BLOCK, U+2584 LOWER HALF BLOCK,
/// U+2588 FULL BLOCK) to display two rows per character line.
use image::Luma;
use qrcode::QrCode;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::theme;

/// State for the QR code view screen.
#[derive(Debug, Clone, Default)]
pub struct QrViewState {
    /// User name displayed as title
    pub user_name: String,
    /// The vless:// URL to encode and display
    pub vless_url: String,
    /// Pre-rendered QR code lines (cached to avoid re-encoding each frame)
    pub qr_lines: Vec<String>,
    /// Error message if QR generation failed
    pub error: Option<String>,
}

impl QrViewState {
    /// Open QR view for a given user name and vless URL.
    pub fn open(&mut self, user_name: String, vless_url: String) {
        self.user_name = user_name;
        match render_qr_to_lines(&vless_url) {
            Ok(lines) => {
                self.qr_lines = lines;
                self.error = None;
            }
            Err(e) => {
                self.qr_lines.clear();
                self.error = Some(e);
            }
        }
        self.vless_url = vless_url;
    }

    /// Clear QR view state.
    pub fn close(&mut self) {
        self.user_name.clear();
        self.vless_url.clear();
        self.qr_lines.clear();
        self.error = None;
    }
}

/// Draw the QR code view screen.
pub fn draw(state: &QrViewState, frame: &mut ratatui::Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border_style())
        .title(Span::styled(
            format!(" QR: {} ", state.user_name),
            theme::secondary_style(),
        ))
        .style(theme::text_style());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let Some(ref err) = state.error {
        let error_msg = Paragraph::new(Line::from(Span::styled(
            format!("Error: {}", err),
            theme::alert_style(),
        )))
        .alignment(Alignment::Center);
        frame.render_widget(error_msg, inner);
        return;
    }

    if state.qr_lines.is_empty() {
        let empty_msg = Paragraph::new(Line::from(Span::styled(
            "No QR code to display",
            theme::muted_style(),
        )))
        .alignment(Alignment::Center);
        frame.render_widget(empty_msg, inner);
        return;
    }

    // Layout: QR code centered, then URL below
    let qr_height = state.qr_lines.len() as u16;
    // Reserve 3 lines for URL display below QR (1 blank + 2 for URL)
    let url_section_height: u16 = 3;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(qr_height.max(1)),      // QR area
            Constraint::Length(url_section_height), // URL area
        ])
        .split(inner);

    // Render QR code centered
    let qr_lines: Vec<Line> = state
        .qr_lines
        .iter()
        .map(|line| Line::from(Span::styled(line.clone(), theme::text_style())))
        .collect();

    // Vertically center the QR code within its area
    let v_pad = if chunks[0].height > qr_height {
        (chunks[0].height - qr_height) / 2
    } else {
        0
    };
    let qr_area = Rect {
        x: chunks[0].x,
        y: chunks[0].y + v_pad,
        width: chunks[0].width,
        height: qr_height.min(chunks[0].height.saturating_sub(v_pad)),
    };

    let qr_paragraph = Paragraph::new(qr_lines).alignment(Alignment::Center);
    frame.render_widget(qr_paragraph, qr_area);

    // Render vless URL below QR code
    let url_lines = vec![
        Line::from(""),
        Line::from(Span::styled(&state.vless_url, theme::secondary_style())),
    ];
    let url_paragraph = Paragraph::new(url_lines)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: false });
    frame.render_widget(url_paragraph, chunks[1]);
}

/// Render a QR code as a vector of strings using unicode half-block characters.
///
/// Each string represents one line of terminal output.
/// Uses "██" for dark modules and "  " for light modules,
/// with two QR rows packed into each terminal row using half-block characters:
/// - "▀" (U+2580) = top dark, bottom light
/// - "▄" (U+2584) = top light, bottom dark
/// - "█" (U+2588) = both dark
/// - " " = both light
///
/// Includes a 1-module quiet zone border.
pub fn render_qr_to_lines(data: &str) -> Result<Vec<String>, String> {
    let code = QrCode::new(data.as_bytes()).map_err(|e| format!("QR encode error: {}", e))?;
    let modules = code.to_colors();
    let width = code.width();
    let height = width; // QR codes are square

    // Add 1-module quiet zone on each side
    let padded_width = width + 2;
    let padded_height = height + 2;

    // Build padded grid (true = dark/black, false = light/white)
    let padded = |row: i32, col: i32| -> bool {
        if row < 0 || col < 0 || row >= height as i32 || col >= width as i32 {
            false // quiet zone is light
        } else {
            modules[row as usize * width + col as usize] == qrcode::Color::Dark
        }
    };

    let mut lines = Vec::new();

    // Process two rows at a time
    let mut y = -1i32; // start at -1 for top quiet zone
    while y < padded_height as i32 - 1 {
        let top_row = y;
        let bottom_row = y + 1;
        let mut line = String::new();

        for x in -1..padded_width as i32 - 1 {
            let top = padded(top_row, x);
            let bottom = padded(bottom_row, x);

            match (top, bottom) {
                (true, true) => line.push('\u{2588}'),  // █ full block
                (true, false) => line.push('\u{2580}'), // ▀ upper half
                (false, true) => line.push('\u{2584}'), // ▄ lower half
                (false, false) => line.push(' '),       // space
            }
        }

        lines.push(line);
        y += 2;
    }

    // If odd number of padded rows, handle the last row
    if padded_height % 2 != 0 {
        let last_row = padded_height as i32 - 2; // -1 for quiet zone offset
        let mut line = String::new();
        for x in -1..padded_width as i32 - 1 {
            if padded(last_row, x) {
                line.push('\u{2580}'); // ▀ upper half (bottom is quiet zone = light)
            } else {
                line.push(' ');
            }
        }
        lines.push(line);
    }

    Ok(lines)
}

/// Render a QR code as PNG image bytes.
///
/// Returns the PNG data as a `Vec<u8>` suitable for sending via Telegram or writing to a file.
/// The `scale` parameter controls the number of pixels per QR module (default: 8).
pub fn render_qr_to_png(data: &str, scale: u32) -> Result<Vec<u8>, String> {
    let code = QrCode::new(data.as_bytes()).map_err(|e| format!("QR encode error: {}", e))?;
    let image = code
        .render::<Luma<u8>>()
        .quiet_zone(true)
        .min_dimensions(scale * 21, scale * 21) // ensure minimum size
        .build();

    let mut png_bytes: Vec<u8> = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut png_bytes);
    image::ImageEncoder::write_image(
        encoder,
        image.as_raw(),
        image.width(),
        image.height(),
        image::ExtendedColorType::L8,
    )
    .map_err(|e| format!("PNG encode error: {}", e))?;

    Ok(png_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn test_render_qr_produces_output() {
        let lines = render_qr_to_lines("test").unwrap();
        assert!(!lines.is_empty());
        // All lines should have the same width
        let width = lines[0].chars().count();
        for line in &lines {
            assert_eq!(line.chars().count(), width);
        }
    }

    #[test]
    fn test_render_qr_contains_block_chars() {
        let lines = render_qr_to_lines("hello").unwrap();
        let all_text: String = lines.join("");
        // Should contain at least some block characters
        assert!(
            all_text.contains('\u{2588}')
                || all_text.contains('\u{2580}')
                || all_text.contains('\u{2584}')
        );
    }

    #[test]
    fn test_render_qr_vless_url() {
        let url = "vless://550e8400-e29b-41d4-a716-446655440000@1.2.3.4:443?encryption=none&flow=xtls-rprx-vision&type=tcp&security=reality&sni=www.googletagmanager.com&fp=chrome&pbk=testkey&sid=abcd1234#TestUser";
        let lines = render_qr_to_lines(url).unwrap();
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_render_qr_empty_string() {
        // Even empty string should produce a valid QR code
        let lines = render_qr_to_lines("").unwrap();
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_render_qr_unicode_content() {
        let lines = render_qr_to_lines("Hello, мир!").unwrap();
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_render_qr_consistent_width() {
        let lines = render_qr_to_lines("consistency test").unwrap();
        if lines.len() > 1 {
            let first_width = lines[0].chars().count();
            for (i, line) in lines.iter().enumerate() {
                assert_eq!(
                    line.chars().count(),
                    first_width,
                    "Line {} has different width",
                    i
                );
            }
        }
    }

    // --- QrViewState tests ---

    #[test]
    fn test_qr_view_state_default() {
        let state = QrViewState::default();
        assert!(state.user_name.is_empty());
        assert!(state.vless_url.is_empty());
        assert!(state.qr_lines.is_empty());
        assert!(state.error.is_none());
    }

    #[test]
    fn test_qr_view_state_open() {
        let mut state = QrViewState::default();
        state.open(
            "alice".to_string(),
            "vless://uuid@1.2.3.4:443?test=1#alice".to_string(),
        );
        assert_eq!(state.user_name, "alice");
        assert!(state.vless_url.contains("vless://"));
        assert!(!state.qr_lines.is_empty());
        assert!(state.error.is_none());
    }

    #[test]
    fn test_qr_view_state_close() {
        let mut state = QrViewState::default();
        state.open("alice".to_string(), "test-url".to_string());
        assert!(!state.qr_lines.is_empty());
        state.close();
        assert!(state.user_name.is_empty());
        assert!(state.vless_url.is_empty());
        assert!(state.qr_lines.is_empty());
        assert!(state.error.is_none());
    }

    #[test]
    fn test_qr_view_state_open_caches_lines() {
        let mut state = QrViewState::default();
        let url = "vless://550e8400@1.2.3.4:443?encryption=none#TestUser";
        state.open("TestUser".to_string(), url.to_string());
        // QR lines should be pre-rendered
        assert!(!state.qr_lines.is_empty());
        // All lines same width
        let width = state.qr_lines[0].chars().count();
        for line in &state.qr_lines {
            assert_eq!(line.chars().count(), width);
        }
    }

    #[test]
    fn test_draw_qr_view_renders_without_panic() {
        let backend = TestBackend::new(80, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = QrViewState::default();
        state.open(
            "alice".to_string(),
            "vless://uuid@1.2.3.4:443?test=1#alice".to_string(),
        );
        terminal
            .draw(|frame| {
                draw(&state, frame, frame.area());
            })
            .unwrap();
    }

    #[test]
    fn test_draw_qr_view_empty_state() {
        let backend = TestBackend::new(80, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = QrViewState::default();
        terminal
            .draw(|frame| {
                draw(&state, frame, frame.area());
            })
            .unwrap();
    }

    #[test]
    fn test_draw_qr_view_with_error() {
        let backend = TestBackend::new(80, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = QrViewState {
            user_name: "bob".to_string(),
            vless_url: String::new(),
            qr_lines: Vec::new(),
            error: Some("test error".to_string()),
        };
        terminal
            .draw(|frame| {
                draw(&state, frame, frame.area());
            })
            .unwrap();
    }

    #[test]
    fn test_draw_qr_view_small_terminal() {
        let backend = TestBackend::new(20, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = QrViewState::default();
        state.open("alice".to_string(), "short".to_string());
        terminal
            .draw(|frame| {
                draw(&state, frame, frame.area());
            })
            .unwrap();
    }

    // --- PNG rendering tests ---

    #[test]
    fn test_render_qr_to_png_produces_valid_png() {
        let png = render_qr_to_png("test", 8).unwrap();
        // PNG files start with the magic bytes
        assert!(png.len() > 8);
        assert_eq!(&png[..4], &[0x89, 0x50, 0x4E, 0x47]); // \x89PNG
    }

    #[test]
    fn test_render_qr_to_png_vless_url() {
        let url = "vless://550e8400-e29b-41d4-a716-446655440000@1.2.3.4:443?encryption=none&flow=xtls-rprx-vision&type=tcp&security=reality&sni=www.googletagmanager.com&fp=chrome&pbk=testkey&sid=abcd1234#TestUser";
        let png = render_qr_to_png(url, 8).unwrap();
        assert!(png.len() > 100); // should be a reasonable size
        assert_eq!(&png[..4], &[0x89, 0x50, 0x4E, 0x47]);
    }

    #[test]
    fn test_render_qr_to_png_different_scales() {
        let small = render_qr_to_png("test", 4).unwrap();
        let large = render_qr_to_png("test", 16).unwrap();
        // Larger scale should produce larger PNG
        assert!(large.len() > small.len());
    }

    #[test]
    fn test_render_qr_to_png_empty_string() {
        let png = render_qr_to_png("", 8).unwrap();
        assert_eq!(&png[..4], &[0x89, 0x50, 0x4E, 0x47]);
    }

    #[test]
    fn test_render_qr_to_png_unicode() {
        let png = render_qr_to_png("Hello, мир!", 8).unwrap();
        assert_eq!(&png[..4], &[0x89, 0x50, 0x4E, 0x47]);
    }
}
