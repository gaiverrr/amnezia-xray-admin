/// QR code rendering as unicode block characters for terminal display.
///
/// Uses the `qrcode` crate to generate QR codes and renders them
/// using unicode half-block characters (U+2580 UPPER HALF BLOCK, U+2584 LOWER HALF BLOCK,
/// U+2588 FULL BLOCK) to display two rows per character line.

use qrcode::QrCode;

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
                (true, true) => line.push('\u{2588}'),   // █ full block
                (true, false) => line.push('\u{2580}'),  // ▀ upper half
                (false, true) => line.push('\u{2584}'),  // ▄ lower half
                (false, false) => line.push(' '),        // space
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
