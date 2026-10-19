//! QR code generation and pixel rasterization.

use qrcode::QrCode;
use qrcode::types::Color;

/// Raw RGBA bitmap generated for a QR code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QrPixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Generates crisp RGBA pixel bytes containing the QR code for `payload`.
///
/// The QR code includes standard 4-module quiet-zone padding. Modules are scaled
/// up cleanly so the resulting bitmap has crisp pixel edges.
pub fn generate_qr_pixels(payload: &str, target_size: u32) -> anyhow::Result<QrPixels> {
    let code = QrCode::new(payload.as_bytes())?;
    let qr_width = code.width();
    let quiet_zone = 4;
    let total_modules = qr_width + quiet_zone * 2;
    let scale = (target_size / total_modules as u32).max(4);
    let img_size = total_modules as u32 * scale;

    let mut rgba = vec![255u8; (img_size * img_size * 4) as usize];

    let colors = code.to_colors();
    for y in 0..qr_width {
        for x in 0..qr_width {
            if colors[y * qr_width + x] == Color::Dark {
                let start_px = (x + quiet_zone) as u32 * scale;
                let start_py = (y + quiet_zone) as u32 * scale;
                for py in start_py..start_py + scale {
                    for px in start_px..start_px + scale {
                        let idx = ((py * img_size + px) * 4) as usize;
                        rgba[idx] = 0; // R
                        rgba[idx + 1] = 0; // G
                        rgba[idx + 2] = 0; // B
                        rgba[idx + 3] = 255; // A
                    }
                }
            }
        }
    }

    Ok(QrPixels {
        width: img_size,
        height: img_size,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_generates_pixels_with_expected_dimensions() {
        let pixels = generate_qr_pixels("immermemo://import#test", 240).unwrap();
        assert!(pixels.width > 0);
        assert_eq!(pixels.width, pixels.height);
        assert_eq!(pixels.rgba.len(), (pixels.width * pixels.height * 4) as usize);
        // Verify background is white and at least some pixels are black
        assert!(pixels.rgba.iter().any(|&b| b == 0));
        assert!(pixels.rgba.iter().any(|&b| b == 255));
    }
}
