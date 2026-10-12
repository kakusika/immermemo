//! Authentication secrets, token storage, and SSH key management.
//!
//! Encapsulates:
//! - [`TokenStore`]: Personal Access Token persistence abstraction and [`PlainFileTokenStore`].
//! - [`SshKeyStore`]: In-app Ed25519 key generation and OpenSSH format persistence.
//! - [`token_blob`]: Encrypted token packaging for cross-device transfer.

pub mod ssh;
pub mod token;
pub mod token_blob;

pub use ssh::SshKeyStore;
pub use token::{PlainFileTokenStore, TokenStore, TokenStoreFactory};
