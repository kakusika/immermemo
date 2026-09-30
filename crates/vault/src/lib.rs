//! Vault management, note operations, local storage, editor history, and credentials.
//!
//! This crate encapsulates all domain logic for an Immermemo vault that does not depend
//! on any specific UI framework (such as Slint).
//!
//! Key components:
//! - [`notes`]: Scanning, creating, renaming, deleting, and full-text searching notes (`.tmt`).
//! - [`appdata`]: Persistent application metadata (vault registry, git storage path, settings).
//! - [`history`]: Undo / redo history stack for note editing backed by Loro.
//! - [`credentials`]: Pluggable authentication token storage ([`TokenStore`]).
//! - [`token_blob`]: Format definition and validation for encrypted token payloads.

pub mod appdata;
pub mod credentials;
pub mod history;
pub mod notes;
pub mod token_blob;

pub use appdata::AppData;
pub use credentials::{PlainFileTokenStore, TokenStore, TokenStoreFactory};
pub use history::History;
pub use notes::{SearchResult, display_name, has_conflict_marker};
