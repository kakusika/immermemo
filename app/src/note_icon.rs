//! A note's declared icon and banner, resolved into the views the UI draws.

use std::path::Path;

use immermemo_media::Preset;
use immermemo_render_tomet::{NoteBanner, NoteIcon, text_icon_cells};
use slint::{ModelRc, SharedString, VecModel};

use crate::thumbs::MediaBase;
use crate::{NoteBannerView, NoteIconKind, NoteIconView};

/// The view for the icon of the note at `note_rel` in `vault_dir`. An image
/// icon shows its thumbnail once there is one (see [`crate::thumbs`]); until
/// then, or if the reference does not lead anywhere readable, the view
/// carries an empty picture.
pub fn resolve_view(icon: Option<&NoteIcon>, vault_dir: &Path, note_rel: &Path) -> NoteIconView {
    let picture = match icon {
        Some(NoteIcon::Image(reference)) => {
            MediaBase::new(vault_dir, note_rel).picture(reference, Preset::Thumb)
        }
        _ => slint::Image::default(),
    };
    icon_view(icon, picture)
}

/// The view for `icon`. `picture` is the already-produced thumbnail for an
/// [`NoteIcon::Image`] (an empty image while it is still being produced or
/// if it could not be); it is ignored for every other kind.
///
/// A glyph the vendored icon packages don't have, and a text icon with
/// nothing visible in it, resolve to an empty view -- the caller's
/// `fallback` glyph stands in, same as a note with no icon at all.
pub fn icon_view(icon: Option<&NoteIcon>, picture: slint::Image) -> NoteIconView {
    let empty = NoteIconView::default;
    match icon {
        None => empty(),
        Some(NoteIcon::Glyph { name, pkg }) => {
            let glyph = crate::render::icon_image(name, pkg);
            if glyph.size().width == 0 {
                return empty();
            }
            NoteIconView {
                kind: NoteIconKind::Glyph,
                glyph,
                ..empty()
            }
        }
        Some(NoteIcon::Text(text)) => {
            let cells = text_icon_cells(text);
            if cells.is_empty() {
                return empty();
            }
            let cells: Vec<SharedString> = cells.into_iter().map(SharedString::from).collect();
            NoteIconView {
                kind: NoteIconKind::Text,
                cells: ModelRc::new(VecModel::from(cells)),
                ..empty()
            }
        }
        Some(NoteIcon::Image(_)) => NoteIconView {
            kind: NoteIconKind::Picture,
            picture,
            ..empty()
        },
    }
}

/// The view for the banner of the note at `note_rel` in `vault_dir`: its
/// picture once there is one (see [`crate::thumbs`]), and where to crop it
/// (centred unless `banner-y` says otherwise).
pub fn banner_view(
    banner: Option<&NoteBanner>,
    vault_dir: &Path,
    note_rel: &Path,
) -> NoteBannerView {
    let Some(banner) = banner else {
        return NoteBannerView::default();
    };
    NoteBannerView {
        declared: true,
        picture: MediaBase::new(vault_dir, note_rel).picture(&banner.reference, Preset::Banner),
        focus_y: banner.focus_y.map_or(0.5, |y| y / 100.0),
    }
}
