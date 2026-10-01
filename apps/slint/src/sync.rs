//! Runs one `Vault::sync` for the window, off the UI thread.
//!
//! Deliberately knows nothing about where credentials come from -- that is
//! `crate::credentials`' job -- or where the gitdir lives -- that is
//! `crate::appdata`'s. This module is just the call into `immermemo_sync`.

use std::path::Path;

use immermemo_sync::{CertificateVerifier, CredentialProvider, SyncReport, Vault};

/// Runs a sync against `remote` and returns the resulting `SyncReport`.
pub fn run(
    vault_dir: &Path,
    gitdir: &Path,
    remote: &str,
    credentials: &dyn CredentialProvider,
    certificate_verifier: Option<Box<dyn CertificateVerifier>>,
) -> anyhow::Result<SyncReport> {
    if !gitdir.exists() {
        std::fs::create_dir_all(gitdir)?;
    }
    let mut vault = Vault::open(vault_dir, gitdir)?;
    vault.set_certificate_verifier(certificate_verifier);
    vault.set_remote(remote)?;
    vault.sync(credentials)
}

#[cfg(test)]
mod tests {
    use super::*;
    use immermemo_vault::appdata::AppData;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Local transport (a plain filesystem path) never invokes the
    /// credentials callback, so these tests never need real credentials.
    struct NoCredentials;
    impl CredentialProvider for NoCredentials {
        fn credentials(&self, _remote_url: &str) -> anyhow::Result<git2::Cred> {
            anyhow::bail!("not expected to be called for a local transport")
        }
    }

    /// A bare repository standing in for GitHub/Gitea: a plain filesystem
    /// path never invokes the credentials callback, so no ssh is involved.
    fn bare_remote() -> TempDir {
        let dir = TempDir::new().unwrap();
        git2::Repository::init_bare(dir.path()).unwrap();
        dir
    }

    /// One "device": a notes folder plus its own app-data base beside it.
    struct Device {
        notes: TempDir,
        app_data: AppData,
        _app_data_dir: TempDir,
    }

    impl Device {
        fn new() -> Self {
            let app_data_dir = TempDir::new().unwrap();
            Self {
                notes: TempDir::new().unwrap(),
                app_data: AppData::new(app_data_dir.path().to_owned()),
                _app_data_dir: app_data_dir,
            }
        }

        fn write(&self, name: &str, text: &str) {
            std::fs::write(self.notes.path().join(name), text).unwrap();
        }

        fn read(&self, name: &str) -> String {
            std::fs::read_to_string(self.notes.path().join(name)).unwrap()
        }

        fn sync(&self, remote: &TempDir) -> anyhow::Result<Vec<PathBuf>> {
            let gitdir = self.app_data.gitdir(self.notes.path())?;
            let report = run(
                self.notes.path(),
                &gitdir,
                remote.path().to_str().unwrap(),
                &NoCredentials,
                None,
            )?;
            Ok(report
                .notes_needing_resolution
                .into_iter()
                .map(|p| self.notes.path().join(p))
                .collect())
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
    fn in_tree_git_repository_is_used_when_already_present() {
        let remote = bare_remote();
        let a = Device::new();
        // User clones or inits repo in the notes directory
        git2::Repository::init(a.notes.path()).unwrap();

        a.write("note.tmt", "In-tree repo\n");
        assert!(a.sync(&remote).unwrap().is_empty());

        // .git is inside notes folder
        assert!(a.notes.path().join(".git").exists());

        // Private app-data git directory was NOT created
        let private_vault_git = a.app_data.base().join("vaults");
        let private_has_git = if private_vault_git.exists() {
            std::fs::read_dir(&private_vault_git)
                .unwrap()
                .any(|e| e.unwrap().path().join("git").exists())
        } else {
            false
        };
        assert!(
            !private_has_git,
            "Private git directory should not be created when in-tree .git exists"
        );

        // The commit exists in the in-tree .git repo
        let repo = git2::Repository::open(a.notes.path()).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.summary(), Some("sync"));
    }
}
