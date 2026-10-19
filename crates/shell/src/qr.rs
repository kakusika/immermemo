//! QR code rasterization into Slint `Image` buffers.

use immermemo_core::generate_qr_pixels;
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// Generates a crisp Slint [`Image`] containing the QR code for `payload`.
///
/// Wraps [`generate_qr_pixels`] from `immermemo-core` into a Slint image buffer.
pub fn generate_qr_image(payload: &str, target_size: u32) -> anyhow::Result<Image> {
    let qr = generate_qr_pixels(payload, target_size)?;
    let mut pixel_buffer = SharedPixelBuffer::<Rgba8Pixel>::new(qr.width, qr.height);
    pixel_buffer.make_mut_bytes().copy_from_slice(&qr.rgba);
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
