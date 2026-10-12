//! Core state management, session handling, note navigation, and directory structures.
//!
//! Independent of Slint or graphical UI frameworks, allowing lightweight,
//! fast compilation and standalone unit testing of note application workflows.

pub mod directory;
pub mod navigation;
pub mod session;

pub use directory::{
    BreadcrumbSegment, DirectoryEntry, TitledNote, build_breadcrumb, build_folder_entries,
};
pub use navigation::BackForwardStack;
pub use session::{
    FilteredListOutcome, RefreshListOutcome, Session, TokenStoreFactory, vault_display_name,
    vault_path_from_input,
};
