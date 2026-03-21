use ratatui::style::{Color, Modifier, Style};

/// Hacker/cyberpunk color palette
pub const BG: Color = Color::Rgb(10, 10, 10); // #0a0a0a near-black
pub const PRIMARY: Color = Color::Rgb(0, 255, 65); // #00ff41 matrix green
pub const SECONDARY: Color = Color::Rgb(0, 212, 255); // #00d4ff cyan
pub const ACCENT: Color = Color::Rgb(255, 0, 255); // #ff00ff magenta
pub const ALERT: Color = Color::Rgb(255, 0, 64); // #ff0040 neon red
pub const MUTED: Color = Color::Rgb(68, 68, 68); // #444444 dark gray
pub const SUCCESS: Color = Color::Rgb(0, 255, 65); // #00ff41 green
pub const BORDER: Color = Color::Rgb(26, 58, 26); // #1a3a1a dark green

pub fn title_style() -> Style {
    Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD)
}

pub fn text_style() -> Style {
    Style::default().fg(PRIMARY).bg(BG)
}

pub fn muted_style() -> Style {
    Style::default().fg(MUTED).bg(BG)
}

pub fn accent_style() -> Style {
    Style::default().fg(ACCENT).bg(BG)
}

pub fn alert_style() -> Style {
    Style::default().fg(ALERT).bg(BG)
}

pub fn secondary_style() -> Style {
    Style::default().fg(SECONDARY).bg(BG)
}

pub fn status_style() -> Style {
    Style::default().fg(SECONDARY).bg(Color::Rgb(5, 5, 5))
}

pub fn border_style() -> Style {
    Style::default().fg(BORDER).bg(BG)
}

pub fn selected_style() -> Style {
    Style::default()
        .fg(Color::Rgb(10, 10, 10))
        .bg(PRIMARY)
        .add_modifier(Modifier::BOLD)
}

pub fn header_style() -> Style {
    Style::default()
        .fg(PRIMARY)
        .bg(Color::Rgb(5, 15, 5))
        .add_modifier(Modifier::BOLD)
}

pub const LOGO: &str = r#"
 ▄▀█ ▀▄▀ ▄▀█ █▀▄ █▀▄▀█ █ █▄░█
 █▀█ █░█ █▀█ █▄▀ █░▀░█ █ █░▀█"#;

pub const APP_NAME: &str = "amnezia-xray-admin";
pub const APP_VERSION: &str = "v0.1.0";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_color_values() {
        assert_eq!(BG, Color::Rgb(10, 10, 10));
        assert_eq!(PRIMARY, Color::Rgb(0, 255, 65));
        assert_eq!(SECONDARY, Color::Rgb(0, 212, 255));
        assert_eq!(ACCENT, Color::Rgb(255, 0, 255));
        assert_eq!(ALERT, Color::Rgb(255, 0, 64));
        assert_eq!(MUTED, Color::Rgb(68, 68, 68));
        assert_eq!(BORDER, Color::Rgb(26, 58, 26));
    }

    #[test]
    fn test_styles_have_correct_fg() {
        assert_eq!(title_style().fg, Some(PRIMARY));
        assert_eq!(text_style().fg, Some(PRIMARY));
        assert_eq!(muted_style().fg, Some(MUTED));
        assert_eq!(accent_style().fg, Some(ACCENT));
        assert_eq!(alert_style().fg, Some(ALERT));
        assert_eq!(secondary_style().fg, Some(SECONDARY));
        assert_eq!(status_style().fg, Some(SECONDARY));
        assert_eq!(border_style().fg, Some(BORDER));
    }

    #[test]
    fn test_selected_style_inverts_colors() {
        let style = selected_style();
        assert_eq!(style.bg, Some(PRIMARY));
        assert_eq!(style.fg, Some(Color::Rgb(10, 10, 10)));
    }

    #[test]
    fn test_logo_is_nonempty() {
        assert!(!LOGO.is_empty());
        assert!(LOGO.contains("▄▀█"));
    }

    #[test]
    fn test_app_constants() {
        assert_eq!(APP_NAME, "amnezia-xray-admin");
        assert_eq!(APP_VERSION, "v0.1.0");
    }
}
