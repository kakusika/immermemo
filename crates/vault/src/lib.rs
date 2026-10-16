//! Vault management, note operations, local storage, and device-to-device transfer.
//!
//! This crate encapsulates all domain logic for an Immermemo vault that does not depend
//! on any specific UI framework (such as Slint).
//!
//! Key components:
//! - [`notes`]: Scanning, creating, renaming, deleting, and full-text searching notes (`.tmt`).
//! - [`appdata`]: Persistent application metadata (vault registry, git storage path, settings).
//! - [`transfer`]: Exporting and importing settings and credentials between devices.
//!
//! Undo/redo (`History`) lives in `immermemo-core::editor` instead -- every
//! consumer of it is there, not here; it only sat in this crate as a
//! leftover from the batch extraction that created it, before `editor`
//! existed to hold editor-session state specifically.

pub mod appdata;
pub mod notes;
pub mod size;
pub mod transfer;

pub use appdata::{Account, AppData};
pub use notes::{SearchResult, display_name, has_conflict_marker};
pub use size::{compute_vault_size, format_vault_size};
pub use transfer::{
    ApplyResult, ExportOptions, InterfaceTransfer, SyncTransfer, TransferPayload, apply_payload,
    build_payload, decode_payload, encode_payload,
};
