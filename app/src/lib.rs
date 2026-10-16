pub mod action;
pub mod avatar;
mod callbacks;
pub mod cli;
mod note_icon;
mod platform;
mod provider;
pub mod qr;
pub mod render;
mod session;
mod startup;
pub mod theme;
mod thumbs;
pub mod transfer;

pub use action::apply_bottom_bar_config;
pub use immermemo_core::{StartVault, TokenStore, TokenStoreFactory};
pub use platform::cert;
pub use startup::{desktop_app_data_dir, run, token_path_in};
pub use thumbs::MediaBase;
pub use transfer::handle_incoming_import_uri;
#[cfg(test)]
pub(crate) use transfer::open_import_preview;

slint::include_modules!();

#[cfg(test)]
mod tests;
