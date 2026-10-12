//! Core state management, session handling, note navigation, and directory structures.
//!
//! Independent of Slint or graphical UI frameworks, allowing lightweight,
//! fast compilation and standalone unit testing of note application workflows.

pub mod directory;
pub mod editor;
pub mod navigation;
pub mod session;

// Domain facade re-exports:
pub use immermemo_identity::DeviceIdentity;
pub use immermemo_index::{IndexedNote, NoteIndex};
pub use immermemo_sync::{
    CertificateVerifier, CommitSummary, ConflictSide, CredentialProvider, FileRevision,
    SyncReport, Vault, tls,
};
pub use immermemo_credentials::{
    PlainFileTokenStore, SshKeyStore, TokenStore, TokenStoreFactory, token_blob,
};
pub use immermemo_vault::appdata::AppData;
pub mod notes {
    pub use immermemo_vault::notes::*;
}
pub mod transfer {
    pub use immermemo_vault::transfer::*;
}
pub use tomet::merge::{CONFLICT_MARKER, ConflictResolution};

pub use directory::{
    BreadcrumbSegment, DirectoryEntry, TitledNote, build_breadcrumb, build_folder_entries,
};
pub use editor::{
    ConflictResolved, ConflictSheetState, EditorState, History, NoteStats, OpenedNote,
    Restored, RestoredNote, RestoredVersion, note_stats, set_note_icon,
};
pub use navigation::BackForwardStack;
pub use session::{
    FilteredListOutcome, RefreshListOutcome, Session, vault_display_name,
    vault_path_from_input,
};
