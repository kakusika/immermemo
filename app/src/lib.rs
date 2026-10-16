pub mod action;
pub mod avatar;
mod callbacks;
mod platform;
mod provider;
pub mod qr;
pub mod render;
mod session;
mod startup;
pub mod theme;
pub mod transfer;

pub use action::apply_bottom_bar_config;
pub use immermemo_core::{TokenStore, TokenStoreFactory};
pub use platform::cert;
pub use provider::update_provider_ui_state;
pub use startup::{desktop_app_data_dir, run, token_path_in};
pub use transfer::handle_incoming_import_uri;
#[cfg(test)]
pub(crate) use transfer::open_import_preview;

slint::include_modules!();

#[cfg(test)]
mod tests;
