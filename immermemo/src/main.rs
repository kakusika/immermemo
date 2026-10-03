use std::path::PathBuf;
use std::sync::Arc;

use immermemo::TokenStoreFactory;
use immermemo::credentials::{PlainFileTokenStore, TokenStore};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let default_vault_dir = PathBuf::from(
        args.next()
            .ok_or_else(|| anyhow::anyhow!("usage: immermemo <notes-dir>"))?,
    );
    let app_data = immermemo::desktop_app_data_dir()?;
    // Plaintext, unlike Android's Keystore-backed store: fine for trying the
    // sync loop out on desktop, not for anything real.
    let base = app_data.clone();
    let token_store_for: TokenStoreFactory = Box::new(move |vault_dir| {
        let path = immermemo::token_path_in(&base, vault_dir)?;
        Ok(Arc::new(PlainFileTokenStore::new(path)) as Arc<dyn TokenStore>)
    });
    immermemo::run(default_vault_dir, app_data, token_store_for)
}
