//! XHTTP+Reality vless:// URL and QR code rendering.

use crate::error::{AppError, Result};

#[derive(Debug, Clone)]
pub struct XhttpUrlParams {
    pub uuid: String,
    pub host: String,
    pub port: u16,
    pub path: String,
    pub sni: String,
    pub public_key: String,
    pub short_id: String,
    pub name: String,
}

pub fn render_xhttp_url(p: &XhttpUrlParams) -> String {
    let path_encoded = p.path.replace('/', "%2F");
    format!(
        "vless://{uuid}@{host}:{port}?encryption=none&type=xhttp&path={path}&security=reality&sni={sni}&fp=chrome&pbk={pbk}&sid={sid}#{name}",
        uuid = p.uuid,
        host = p.host,
        port = p.port,
        path = path_encoded,
        sni = p.sni,
        pbk = p.public_key,
        sid = p.short_id,
        name = p.name,
    )
}

pub fn render_qr_png(data: &str) -> Result<Vec<u8>> {
    use qrcode::QrCode;
    let code = QrCode::new(data).map_err(|e| AppError::Config(format!("qr encode: {e}")))?;
    let image = code
        .render::<image::Luma<u8>>()
        .min_dimensions(200, 200)
        .build();
    let mut buf = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| AppError::Config(format!("qr png write: {e}")))?;
    Ok(buf.into_inner())
}

pub fn render_qr_ascii(data: &str) -> String {
    use qrcode::render::unicode;
    use qrcode::QrCode;
    match QrCode::new(data) {
        Ok(code) => code.render::<unicode::Dense1x2>().quiet_zone(true).build(),
        Err(_) => "(qr encoding failed)".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xhttp_url_format_matches_expected() {
        let params = XhttpUrlParams {
            uuid: "00000000-0000-0000-0000-000000000001".into(),
            host: "1.2.3.4".into(),
            port: 443,
            path: "/testpath".into(),
            sni: "www.sberbank.ru".into(),
            public_key: "TEST_PUB".into(),
            short_id: "TESTSID".into(),
            name: "alice".into(),
        };
        let url = render_xhttp_url(&params);
        assert_eq!(
            url,
            "vless://00000000-0000-0000-0000-000000000001@1.2.3.4:443?encryption=none&type=xhttp&path=%2Ftestpath&security=reality&sni=www.sberbank.ru&fp=chrome&pbk=TEST_PUB&sid=TESTSID#alice"
        );
    }

    #[test]
    fn qr_produces_valid_png() {
        let url = "vless://test";
        let png = render_qr_png(url).unwrap();
        // PNG magic header
        assert_eq!(&png[..8], &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
    }

    #[test]
    fn ascii_qr_non_empty() {
        let ascii = render_qr_ascii("vless://test");
        assert!(ascii.len() > 100);
        assert!(ascii.contains('\n'));
    }
}
