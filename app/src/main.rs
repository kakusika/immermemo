use std::sync::Arc;

use immermemo_core::{PlainFileTokenStore, TokenStore, TokenStoreFactory};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{}", immermemo::cli::USAGE);
        return Ok(());
    }
    let launch = immermemo::cli::parse_args(args).map_err(anyhow::Error::msg)?;
    let app_data = immermemo::desktop_app_data_dir()?;
    // Plaintext, unlike Android's Keystore-backed store: fine for trying the
    // sync loop out on desktop, not for anything real.
    let base = app_data.clone();
    let token_store_for: TokenStoreFactory = Box::new(move |vault_dir| {
        let path = immermemo::token_path_in(&base, vault_dir)?;
        Ok(Arc::new(PlainFileTokenStore::new(path)) as Arc<dyn TokenStore>)
    });
    let start = immermemo::StartVault {
        first_run: app_data.join("notes"),
        open: launch.open,
    };
    immermemo::run(start, app_data, token_store_for, None, launch.import_uri)
}
