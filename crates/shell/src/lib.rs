//! Slint UI Shell for immermemo.
//!
//! Owns Slint component definitions, generated bindings, UI view models, and callback wiring.

pub mod action;
pub mod avatar;
pub mod callbacks;
pub mod note_icon;
pub use immermemo_platform as platform;
pub mod provider;
pub mod qr;
pub mod render;
pub mod session;
pub mod startup;
pub use immermemo_core::system_info;
pub mod theme;
pub mod thumbs;
pub mod transfer;

pub use action::apply_bottom_bar_config;
pub use immermemo_core::{StartVault, TokenStore, TokenStoreFactory};
pub use immermemo_shell_state as state;
pub use immermemo_shell_state::*;
pub use platform::cert;
pub use startup::{desktop_app_data_dir, run, token_path_in};
pub use thumbs::MediaBase;
pub use transfer::handle_incoming_import_uri;
#[cfg(test)]
pub(crate) use transfer::open_import_preview;

slint::include_modules!();

#[cfg(test)]
mod tests;
