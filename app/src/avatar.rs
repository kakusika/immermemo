//! Provider account avatar loading, local caching, and background retrieval.

use immermemo_core::{Account, AppData};
use std::collections::HashSet;
use std::io::Read;
use std::sync::{LazyLock, Mutex};

static IN_FLIGHT: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// Decodes raw image bytes into a Slint `Image`.
///
/// Supports PNG, JPEG, WebP (via the `image` crate) as well as SVG data.
pub fn decode_avatar(bytes: &[u8]) -> Option<slint::Image> {
    if bytes.is_empty() {
        return None;
    }

    // Try standard bitmap formats (PNG, JPEG, WebP)
    if let Ok(dynamic_img) = image::load_from_memory(bytes) {
        let rgba = dynamic_img.to_rgba8();
        let (width, height) = rgba.dimensions();
        let mut pixel_buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(width, height);
        pixel_buffer.make_mut_bytes().copy_from_slice(rgba.as_raw());
        return Some(slint::Image::from_rgba8(pixel_buffer));
    }

    // Fallback: check if the bytes represent an SVG document
    let trimmed = bytes.trim_ascii_start();
    if (trimmed.starts_with(b"<svg") || trimmed.starts_with(b"<?xml"))
        && let Ok(img) = slint::Image::load_from_svg_data(bytes)
    {
        return Some(img);
    }

    None
}

/// Returns the cached avatar image for `account` if present, or triggers an
/// asynchronous background download from `account.avatar_url` and returns a default image.
///
/// Returns `(image, has_avatar)` where `has_avatar` is `true` if an avatar was successfully loaded.
pub fn load_or_fetch_avatar(
    app_data: &AppData,
    account: &Account,
    app_weak: slint::Weak<crate::App>,
) -> (slint::Image, bool) {
    let Ok(avatar_path) = app_data.account_avatar_path(&account.id) else {
        return (slint::Image::default(), false);
    };

    if avatar_path.is_file()
        && let Ok(bytes) = std::fs::read(&avatar_path)
        && let Some(img) = decode_avatar(&bytes)
    {
        return (img, true);
    }

    // If no cached avatar exists or reading failed, check for an avatar URL.
    if let Some(url) = &account.avatar_url {
        let mut in_flight = IN_FLIGHT.lock().unwrap();
        if in_flight.insert(account.id.clone()) {
            let account_id = account.id.clone();
            let url = url.clone();
            let dest_path = avatar_path.clone();

            std::thread::spawn(move || {
                let download_res = ureq::get(&url).header("User-Agent", "immermemo").call();

                match download_res {
                    Ok(resp) => {
                        let mut bytes = Vec::new();
                        if resp
                            .into_body()
                            .into_reader()
                            .read_to_end(&mut bytes)
                            .is_ok()
                            && decode_avatar(&bytes).is_some()
                        {
                            if let Some(parent) = dest_path.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            let _ = std::fs::write(&dest_path, &bytes);
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to fetch avatar for {account_id}: {e}");
                    }
                }

                if let Ok(mut in_flight) = IN_FLIGHT.lock() {
                    in_flight.remove(&account_id);
                }

                // Notify UI to refresh accounts and vaults on main event loop.
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(app) = app_weak.upgrade() {
                        let session = crate::session::SESSION.with(|s| s.borrow().clone());
                        if let Some(session) = session {
                            crate::callbacks::refresh_accounts_ui(&app, &session.borrow());
                            crate::session::refresh_vault_list(&app, &session);
                        }
                    }
                });
            });
        }
    }

    (slint::Image::default(), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_avatar_empty_and_invalid() {
        assert!(decode_avatar(&[]).is_none());
        assert!(decode_avatar(b"invalid image bytes").is_none());
    }

    #[test]
    fn decode_avatar_valid_png() {
        let mut png_bytes = Vec::new();
        let img = image::RgbaImage::new(2, 2);
        img.write_to(
            &mut std::io::Cursor::new(&mut png_bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
        let decoded = decode_avatar(&png_bytes);
        assert!(decoded.is_some());
    }

    #[test]
    fn decode_avatar_valid_svg() {
        let svg_bytes = b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\"><circle cx=\"12\" cy=\"12\" r=\"10\"/></svg>";
        let decoded = decode_avatar(svg_bytes);
        assert!(decoded.is_some());
    }
}
