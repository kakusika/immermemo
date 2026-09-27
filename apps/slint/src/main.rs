use std::path::PathBuf;
use std::sync::Arc;

use immermemo_slint::credentials::PlainFileTokenStore;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let vault_dir = PathBuf::from(
        args.next()
            .ok_or_else(|| anyhow::anyhow!("usage: immermemo <notes-dir>"))?,
    );
    let app_data = immermemo_slint::desktop_app_data_dir()?;
    // Plaintext, unlike Android's Keystore-backed store: fine for trying the
    // sync loop out on desktop, not for anything real.
    let token_store = Arc::new(PlainFileTokenStore::new(app_data.join("token")));
    immermemo_slint::run(vault_dir, app_data, token_store)
}
