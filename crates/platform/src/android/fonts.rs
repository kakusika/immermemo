//! Repairs the font fallbacks Slint builds on Android.
//!
//! Slint hands text layout to parley, which asks fontique for fallback fonts.
//! fontique's Android backend derives them from `/system/etc/fonts.xml`, where
//! `NotoSansSymbols-Regular-Subsetted.ttf` carries the common geometric and
//! decorative symbols (`◕`, `✧`, ...) that kaomoji lean on. It is listed in
//! `fonts.xml` as a family with neither `lang` nor `postScriptName`, which
//! fontique reads only into the fallbacks that have one, so the glyphs were
//! unreachable. [`repair_fallbacks`] appends it to the generic families Slint
//! falls back through, after the UI font and the CJK fonts.

use slint::fontique_012::fontique::GenericFamily;

/// The system family holding the symbol fonts.
const SYMBOLS_FAMILY: &str = "Noto Sans Symbols";

/// Applies the fallback repairs. Call once the Slint backend is initialised;
/// they apply to the whole process.
pub fn repair_fallbacks() {
    let mut collection = slint::fontique_012::shared_collection();

    if let Some(symbols) = collection.family_id(SYMBOLS_FAMILY) {
        // The same pair Slint appends to every font stack.
        for generic in [GenericFamily::SansSerif, GenericFamily::SystemUi] {
            collection.append_generic_families(generic, [symbols].into_iter());
        }
    }
}
