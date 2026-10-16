//! Callback wiring modules grouped by feature domain.

pub mod account;
pub mod appearance;
pub mod note;
pub mod selection;
pub mod sync_conflict;
pub mod vault;
pub mod vault_setup;

pub use account::{bind_account_callbacks, refresh_accounts_ui};
pub use appearance::bind_appearance_callbacks;
pub use note::bind_note_callbacks;
pub use selection::bind_selection_callbacks;
pub use sync_conflict::bind_sync_conflict_callbacks;
pub use vault::bind_vault_callbacks;
pub use vault_setup::bind_vault_setup_callbacks;
