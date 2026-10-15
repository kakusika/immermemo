//! QR code rasterization into Slint `Image` buffers.

use qrcode::QrCode;
use qrcode::types::Color;
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// Generates a crisp Slint [`Image`] containing the QR code for `payload`.
///
/// The QR code includes standard 4-module quiet-zone padding. Modules are scaled
/// up cleanly so the resulting bitmap has crisp pixel edges.
pub fn generate_qr_image(payload: &str, target_size: u32) -> anyhow::Result<Image> {
    let code = QrCode::new(payload.as_bytes())?;
    let qr_width = code.width();
    let quiet_zone = 4;
    let total_modules = qr_width + quiet_zone * 2;
    let scale = (target_size / total_modules as u32).max(4);
    let img_size = total_modules as u32 * scale;

    let mut pixel_buffer = SharedPixelBuffer::<Rgba8Pixel>::new(img_size, img_size);
    let pixels = pixel_buffer.make_mut_bytes();

    // Fill background with opaque white.
    pixels.fill(255);

    let colors = code.to_colors();
    for y in 0..qr_width {
        for x in 0..qr_width {
            let color = colors[y * qr_width + x];
            if color == Color::Dark {
                let start_px = (x + quiet_zone) as u32 * scale;
                let start_py = (y + quiet_zone) as u32 * scale;
                for py in start_py..start_py + scale {
                    for px in start_px..start_px + scale {
                        let idx = ((py * img_size + px) * 4) as usize;
                        pixels[idx] = 0; // R
                        pixels[idx + 1] = 0; // G
                        pixels[idx + 2] = 0; // B
                        pixels[idx + 3] = 255; // A
                    }
                }
            }
        }
    }

    Ok(Image::from_rgba8(pixel_buffer))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_generates_image_with_nonzero_dimensions() {
        let image = generate_qr_image("immermemo://import#test", 240).unwrap();
        let size = image.size();
        assert!(size.width > 0);
        assert!(size.height > 0);
    }
}
