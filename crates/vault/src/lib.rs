//! Vault management, note operations, local storage, and credentials.
//!
//! This crate encapsulates all domain logic for an Immermemo vault that does not depend
//! on any specific UI framework (such as Slint).
//!
//! Key components:
//! - [`notes`]: Scanning, creating, renaming, deleting, and full-text searching notes (`.tmt`).
//! - [`appdata`]: Persistent application metadata (vault registry, git storage path, settings).
//! - [`credentials`]: Pluggable authentication token storage ([`TokenStore`]).
//! - [`token_blob`]: Format definition and validation for encrypted token payloads.
//!
//! Undo/redo (`History`) lives in `immermemo-editor` instead -- every
//! consumer of it is there, not here; it only sat in this crate as a
//! leftover from the batch extraction that created it, before `editor`
//! existed to hold editor-session state specifically.

pub mod appdata;
pub mod credentials;
pub mod notes;
pub mod token_blob;

pub use appdata::AppData;
pub use credentials::{PlainFileTokenStore, TokenStore, TokenStoreFactory};
pub use notes::{SearchResult, display_name, has_conflict_marker};
