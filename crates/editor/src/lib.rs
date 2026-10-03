//! The editor's state-management logic: what note is open, its undo
//! history, and conflict resolution -- split out of `immermemo` (the
//! Slint app crate) so this logic can be unit-tested and read on its
//! own, independent of the Slint window tree. Note-body classification
//! for the view-mode display (what `classify`/`flow` used to be here)
//! moved into `immermemo::render` instead: that's render *policy* tied
//! to the Slint seam it feeds, not editor state, and the common way it
//! changes (giving a new `@xxx` a look) always touches that seam anyway,
//! so a crate boundary between them bought nothing.
//!
//! This crate never touches `App` (the Slint type `slint::include_modules!()`
//! generates from `immermemo/ui/app.slint`) or any other Slint-generated
//! type. Every function here takes and returns plain data; `immermemo`'s
//! `src/session.rs` is the thin adapter that calls into this crate and
//! applies the result to `App`'s setters. That split exists because `App`
//! is one Slint component tree covering every screen (not just the
//! editor), generated inside `immermemo` itself -- a function here taking
//! `&App` would make `immermemo` depend on a type defined downstream of
//! its own dependency on this crate.
//!
//! `EditorState` ([`state::EditorState`]) holds only what the editor
//! itself owns (the open note's index, its undo history, its loaded
//! revision list). Vault-level state that other screens also read --
//! the note list, each note's conflict flag, the search index -- stays in
//! `immermemo`'s own `Session` and is passed into this crate's functions
//! as plain borrowed data.

mod conflicts;
mod history;
mod note_history;
mod notes;
mod state;
mod stats;

pub use conflicts::{
    ConflictResolved, ConflictSheetState, conflict_sheet_state, resolve_active_conflict,
    resolve_conflict_step,
};
pub use history::{History, Restored};
pub use note_history::{RestoredVersion, load_note_history, restore_note_version};
pub use notes::{
    OpenedNote, RestoredNote, apply_restored, first_conflicted_note_index,
    initial_or_last_note_index, open_note,
};
pub use state::{EditorState, current_path};
pub use stats::{NoteStats, note_stats};
