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
//! Undo/redo (`History`) lives in `immermemo-core::editor` instead -- every
//! consumer of it is there, not here; it only sat in this crate as a
//! leftover from the batch extraction that created it, before `editor`
//! existed to hold editor-session state specifically.

pub mod appdata;
pub mod notes;
pub mod transfer;

pub use appdata::{Account, AppData};
pub use immermemo_credentials::token_blob;
pub use immermemo_credentials::{PlainFileTokenStore, SshKeyStore, TokenStore, TokenStoreFactory};
pub mod credentials {
    pub use immermemo_credentials::*;
}
pub use notes::{SearchResult, display_name, has_conflict_marker};
pub use transfer::{
    ApplyResult, ExportOptions, InterfaceTransfer, SyncTransfer, TransferPayload, apply_payload,
    build_payload, decode_payload, encode_payload,
};
