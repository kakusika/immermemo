//! Which build this is: the commit it was built from, a fingerprint of the
//! uncommitted changes on top of it, and when it was built (UTC) -- the last two
//! only for a release build.
//!
//! It answers one question a phone cannot otherwise: "is the app I am looking
//! at the one I just built?" It is a crate of its own so that the stamp --
//! which differs on every release build -- recompiles only this file. Put in
//! the app's own `build.rs`, it would re-run the whole Slint compile on each
//! build. A release build's script has no input that is ever up to date, by
//! design; a debug build carries no time, so it never forces a rebuild.

/// Short hash of the Git `HEAD` the build was made from, or `unknown` outside a
/// Git checkout. Under `jj`, `HEAD` is the parent of the change being edited.
pub const COMMIT: &str = env!("IMMERMEMO_BUILD_COMMIT");

/// A fingerprint (8 hex digits) of the sources that differed from [`COMMIT`] when
/// the build was made; empty when none did, and for a debug build. The same
/// sources always give the same fingerprint and different sources do not, which
/// is what lets it say whether an edit is in the build: the commit alone cannot,
/// as under `jj` it is the parent of the change being edited and stays put
/// while that change grows.
pub const CHANGES: &str = env!("IMMERMEMO_BUILD_CHANGES");

/// When it was built, `YYYY-MM-DD HH:MM`, in UTC; empty for a debug build.
pub const TIME: &str = env!("IMMERMEMO_BUILD_TIME");

/// The one-line identification shown to the user:
/// `0b9bc064+3fa2c1d9 · 2026-10-16 14:05 UTC` -- the commit, `+` and the
/// fingerprint of the changes on top of it (none when there were none), and the
/// time -- or `debug build`, which says nothing about the sources.
pub fn label() -> String {
    format_label(COMMIT, CHANGES, TIME)
}

fn format_label(commit: &str, changes: &str, time: &str) -> String {
    if time.is_empty() {
        return "debug build".to_owned();
    }
    let changes = if changes.is_empty() {
        String::new()
    } else {
        format!("+{changes}")
    };
    format!("{commit}{changes} · {time} UTC")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_build_is_its_commit_and_time() {
        assert_eq!(
            format_label("0b9bc064", "", "2026-10-16 14:05"),
            "0b9bc064 · 2026-10-16 14:05 UTC"
        );
    }

    #[test]
    fn a_build_with_edits_carries_their_fingerprint() {
        assert_eq!(
            format_label("0b9bc064", "3fa2c1d9", "2026-10-16 14:05"),
            "0b9bc064+3fa2c1d9 · 2026-10-16 14:05 UTC"
        );
    }

    #[test]
    fn a_debug_build_claims_nothing_about_its_sources() {
        assert_eq!(format_label("0b9bc064", "3fa2c1d9", ""), "debug build");
    }

    #[test]
    fn the_stamp_is_filled_in() {
        assert_ne!(COMMIT, "");
        // Tests build in the debug profile; a release build holds a time.
        assert!(
            TIME.is_empty() || TIME.len() == "2026-10-16 14:05".len(),
            "{TIME}"
        );
        assert!(CHANGES.is_empty() || CHANGES.len() == 8, "{CHANGES}");
    }
}
