//! Callback wiring modules grouped by feature domain.

pub mod note;
pub mod sync_conflict;
pub mod vault;

pub use note::bind_note_callbacks;
pub use sync_conflict::bind_sync_conflict_callbacks;
pub use vault::{bind_vault_callbacks, migrate_legacy_remotes, refresh_accounts_ui};
