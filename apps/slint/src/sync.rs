//! Runs one `Vault::sync` for the window, off the UI thread.

use std::path::{Path, PathBuf};

use immermemo_sync::{CredentialProvider, Vault};

/// Desktop credentials: the running ssh-agent for ssh remotes, git's
/// default (no) credentials otherwise. Enough for a trial; a real
/// credential store belongs to the platform layer.
struct DesktopCredentials;

impl CredentialProvider for DesktopCredentials {
    fn credentials(&self, remote_url: &str) -> anyhow::Result<git2::Cred> {
        if remote_url.starts_with("ssh://") || remote_url.contains('@') {
            Ok(git2::Cred::ssh_key_from_agent("git")?)
        } else {
            Ok(git2::Cred::default()?)
        }
    }
}

/// Where the vault's private (bare) git directory lives: outside the notes
/// folder, keyed by the folder's canonical path so two vaults never share one.
fn private_gitdir(vault_dir: &Path) -> anyhow::Result<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .ok_or_else(|| anyhow::anyhow!("neither XDG_DATA_HOME nor HOME is set"))?;
    private_gitdir_under(&base, vault_dir)
}

fn private_gitdir_under(base: &Path, vault_dir: &Path) -> anyhow::Result<PathBuf> {
    let canonical = vault_dir.canonicalize()?;
    let key = canonical.to_string_lossy().replace(['/', '\\'], "_");
    Ok(base.join("immermemo").join("vaults").join(key))
}

/// Returns the notes (absolute paths) that came back with unresolved conflicts.
pub fn run(vault_dir: &Path, remote: &str) -> anyhow::Result<Vec<PathBuf>> {
    run_in(vault_dir, &private_gitdir(vault_dir)?, remote)
}

fn run_in(vault_dir: &Path, gitdir: &Path, remote: &str) -> anyhow::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(gitdir)?;
    let mut vault = Vault::open(vault_dir, gitdir)?;
    vault.set_remote(remote)?;
    let report = vault.sync(&DesktopCredentials)?;
    Ok(report
        .notes_needing_resolution
        .into_iter()
        .map(|p| vault_dir.join(p))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// A bare repository standing in for GitHub/Gitea: a plain filesystem
    /// path never invokes the credentials callback, so no ssh is involved.
    fn bare_remote() -> TempDir {
        let dir = TempDir::new().unwrap();
        git2::Repository::init_bare(dir.path()).unwrap();
        dir
    }

    /// One "device": a notes folder plus the private gitdir base beside it.
    struct Device {
        notes: TempDir,
        gitdir: TempDir,
    }

    impl Device {
        fn new() -> Self {
            Self {
                notes: TempDir::new().unwrap(),
                gitdir: TempDir::new().unwrap(),
            }
        }

        fn write(&self, name: &str, text: &str) {
            std::fs::write(self.notes.path().join(name), text).unwrap();
        }

        fn read(&self, name: &str) -> String {
            std::fs::read_to_string(self.notes.path().join(name)).unwrap()
        }

        fn sync(&self, remote: &TempDir) -> anyhow::Result<Vec<PathBuf>> {
            let gitdir = private_gitdir_under(self.gitdir.path(), self.notes.path())?;
            run_in(self.notes.path(), &gitdir, remote.path().to_str().unwrap())
        }
    }

    #[test]
    fn an_edit_on_one_device_reaches_the_other() {
        let remote = bare_remote();
        let (a, b) = (Device::new(), Device::new());

        a.write("note.tmt", "こんにちは\n");
        assert!(a.sync(&remote).unwrap().is_empty());

        assert!(b.sync(&remote).unwrap().is_empty());
        assert_eq!(b.read("note.tmt"), "こんにちは\n");
    }

    #[test]
    fn concurrent_edits_come_back_as_absolute_conflicted_paths() {
        let remote = bare_remote();
        let (a, b) = (Device::new(), Device::new());
        a.write("note.tmt", "Bring a laptop.\n");
        a.sync(&remote).unwrap();
        b.sync(&remote).unwrap();

        a.write("note.tmt", "Bring a charger.\n");
        a.sync(&remote).unwrap();
        b.write("note.tmt", "Bring a notebook.\n");
        let conflicted = b.sync(&remote).unwrap();

        // The window reopens notes by path, so they must be absolute.
        assert_eq!(conflicted, vec![b.notes.path().join("note.tmt")]);
        assert!(b.read("note.tmt").contains("@mobile.conflict"));
    }

    #[test]
    fn the_git_directory_stays_out_of_the_notes_folder() {
        let remote = bare_remote();
        let a = Device::new();
        a.write("note.tmt", "x\n");
        a.sync(&remote).unwrap();

        let names: Vec<_> = std::fs::read_dir(a.notes.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["note.tmt"]);
    }

    #[test]
    fn each_folder_gets_its_own_gitdir() {
        let base = TempDir::new().unwrap();
        let (one, two) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let a = private_gitdir_under(base.path(), one.path()).unwrap();
        let b = private_gitdir_under(base.path(), two.path()).unwrap();
        assert_ne!(a, b);
        assert_eq!(a, private_gitdir_under(base.path(), one.path()).unwrap());
        assert!(a.starts_with(base.path()));
    }
}
