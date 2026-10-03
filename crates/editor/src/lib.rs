//! The editor's state-management logic: what note is open, its undo
//! history, note-body classification for the view-mode display, and
//! conflict resolution -- split out of `apps/slint` so this logic can be
//! unit-tested and read on its own, independent of the Slint window tree.
//!
//! This crate never touches `App` (the Slint type `slint::include_modules!()`
//! generates from `apps/slint/ui/app.slint`) or any other Slint-generated
//! type. Every function here takes and returns plain data; `apps/slint`'s
//! `src/session.rs` is the thin adapter that calls into this crate and
//! applies the result to `App`'s setters. That split exists because `App`
//! is one Slint component tree covering every screen (not just the
//! editor), generated inside `apps/slint` itself -- a function here taking
//! `&App` would make `apps/slint` depend on a type defined downstream of
//! its own dependency on this crate.
//!
//! `EditorState` ([`state::EditorState`]) holds only what the editor
//! itself owns (the open note's index, its undo history, its loaded
//! revision list). Vault-level state that other screens also read --
//! the note list, each note's conflict flag, the search index -- stays in
//! `apps/slint`'s own `Session` and is passed into this crate's functions
//! as plain borrowed data.

mod classify;
mod conflicts;
mod note_history;
mod notes;
mod state;
mod stats;

pub use classify::{BlockShape, ClassifiedBlock, Tone, classify_body};
pub use conflicts::{
    ConflictResolved, ConflictSheetState, conflict_sheet_state, resolve_active_conflict,
    resolve_conflict_step,
};
pub use note_history::{RestoredVersion, load_note_history, restore_note_version};
pub use notes::{
    OpenedNote, RestoredNote, apply_restored, first_conflicted_note_index,
    initial_or_last_note_index, open_note,
};
pub use state::{EditorState, current_path};
pub use stats::{NoteStats, note_stats};
