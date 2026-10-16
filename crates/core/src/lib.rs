//! Core state management, session handling, note navigation, and directory structures.
//!
//! Independent of Slint or graphical UI frameworks, allowing lightweight,
//! fast compilation and standalone unit testing of note application workflows.
//!
//! This crate is also the one door the app uses into the crates below it
//! (`vault`, `credentials`, `index`, `sync`): the `pub use`s here are the
//! complete list of what the app reaches for from them, so the app never
//! needs to depend on those crates itself. Add a name to the list when the
//! app needs it, not before.

pub mod credentials;
pub mod directory;
pub mod editor;
pub mod format;
pub mod navigation;
pub mod session;
pub mod sync;

pub use format::{format_time_only, format_timestamp};

// The app's view of the crates below this one:
pub use immermemo_credentials::{PlainFileTokenStore, TokenStore, TokenStoreFactory, token_blob};
pub use immermemo_index::{IndexedNote, NoteIndex};
pub use immermemo_sync::{CertificateVerifier, ConflictSide, FileRevision, SyncReport, tls};
pub use immermemo_vault::appdata::{Account, AppData};
pub use immermemo_vault::{compute_vault_size, format_vault_size};
pub mod notes {
    pub use immermemo_vault::notes::{
        SearchResult, create, delete, display_name, has_conflict_marker, rename,
    };
}
pub mod transfer {
    pub use immermemo_vault::transfer::{
        ApplyResult, ExportOptions, InterfaceTransfer, SyncTransfer, TransferPayload,
        apply_payload, build_payload, decode_payload, encode_payload,
    };
}
pub use tomet::merge::{CONFLICT_MARKER, ConflictResolution};

pub use directory::{
    BreadcrumbSegment, DirectoryEntry, TitledNote, build_breadcrumb, build_folder_entries,
};
pub use editor::{
    ConflictResolved, ConflictSheetState, EditorState, History, NoteStats, OpenedNote, Restored,
    RestoredNote, RestoredVersion, note_stats, set_note_icon,
};
pub use navigation::BackForwardStack;
pub use session::{
    ConflictFlags, EditOutcome, FilteredListOutcome, RefreshListOutcome, Session,
    vault_display_name, vault_path_from_input,
};
