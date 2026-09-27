use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let vault_dir = PathBuf::from(
        args.next()
            .ok_or_else(|| anyhow::anyhow!("usage: immermemo <notes-dir> [remote-url]"))?,
    );
    immermemo_slint::run(vault_dir, args.next())
}
