//! Where a vault's private files live: outside the notes folder (which the
//! user's file provider may sync), keyed by the vault's own path so more
//! than one linked folder never collides. Everything under here is this
//! app's own data.
//!
//! Each vault's directory (`base/vaults/<key>/`) holds three things that
//! belong to that vault specifically, and nothing else: its bare git
//! directory, its remote URL, and its access token -- switching vaults
//! switches all three together, since a different vault may point at a
//! completely different host. Which vaults exist at all, and which one is
//! current, are the only things kept at the top level instead: they're
//! properties of the *app*, not of any one vault.
//!
//! Split by responsibility: [`paths`] (where a vault's own files live),
//! [`registry`] (which vaults exist, which is current), [`settings`]
//! (remote URL, last-opened note, editor font size, theme), [`identity`]
//! (where this device's own `immermemo-identity` file lives -- app-level,
//! like `registry`'s `known_vaults`/`current_vault`, not per-vault) --
//! all as `impl AppData` blocks over the one struct defined here.

mod account;
mod identity;
mod paths;
mod registry;
mod settings;
mod ssh;

pub use account::Account;

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct AppData {
    base: PathBuf,
}

impl AppData {
    pub fn new(base: PathBuf) -> Self {
        Self { base }
    }

    /// The app-data root itself -- for a platform that needs to compute a
    /// new vault's path relative to it (Android: a subfolder name).
    pub fn base(&self) -> &Path {
        &self.base
    }

    /// Where the thumbnails of the images notes refer to are cached. Shared by
    /// every vault (one remote image is one entry however many notes use it)
    /// and disposable: nothing here is the only copy of anything, it is
    /// rebuilt from the originals on demand.
    pub fn media_cache_dir(&self) -> PathBuf {
        self.base.join("cache").join("media")
    }
}

fn read_trimmed(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn write(path: &Path, content: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_url_round_trips_and_starts_unset_per_vault() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().to_owned());
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        assert_eq!(data.load_remote(a.path()), None);

        data.save_remote(a.path(), "https://example.invalid/a.git")
            .unwrap();
        assert_eq!(
            data.load_remote(a.path()),
            Some("https://example.invalid/a.git".to_owned())
        );
        // A different vault's remote is independent.
        assert_eq!(data.load_remote(b.path()), None);
    }

    #[test]
    fn each_vault_gets_its_own_gitdir_and_token_path() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().join("appdata"));
        let (one, two) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let a = data.gitdir(one.path()).unwrap();
        let b = data.gitdir(two.path()).unwrap();
        assert_ne!(a, b);
        assert_eq!(a, data.gitdir(one.path()).unwrap());
        assert!(a.starts_with(dir.path().join("appdata")));

        assert_ne!(
            data.token_path(one.path()).unwrap(),
            data.token_path(two.path()).unwrap()
        );
        assert_ne!(
            data.index_path(one.path()).unwrap(),
            data.index_path(two.path()).unwrap()
        );
        // Not nested inside the bare gitdir: that directory belongs to
        // libgit2, nothing else should write into it.
        assert!(!data.token_path(one.path()).unwrap().starts_with(&a));
        assert!(!data.index_path(one.path()).unwrap().starts_with(&a));
    }

    #[test]
    fn known_vaults_starts_empty_and_add_vault_dedupes() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().to_owned());
        assert_eq!(data.known_vaults(), Vec::<PathBuf>::new());

        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        data.add_vault(a.path()).unwrap();
        data.add_vault(b.path()).unwrap();
        data.add_vault(a.path()).unwrap(); // already known, no duplicate
        assert_eq!(
            data.known_vaults(),
            vec![a.path().to_owned(), b.path().to_owned()]
        );
    }

    #[test]
    fn remove_vault_drops_from_list_and_wipes_private_data() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().to_owned());
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        data.add_vault(a.path()).unwrap();
        data.add_vault(b.path()).unwrap();
        data.save_remote(a.path(), "https://example.invalid/a.git")
            .unwrap();
        data.save_current_vault(a.path()).unwrap();

        let a_data_dir = data.vault_data_dir(a.path());
        assert!(a_data_dir.exists());

        data.remove_vault(a.path()).unwrap();
        assert_eq!(data.known_vaults(), vec![b.path().to_owned()]);
        assert!(!a_data_dir.exists());
        // Current vault switched to b.
        assert_eq!(data.load_current_vault(), Some(b.path().to_owned()));
        // The notes directory itself remains untouched.
        assert!(a.path().exists());
    }

    #[test]
    fn current_vault_round_trips_and_starts_unset() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().to_owned());
        assert_eq!(data.load_current_vault(), None);
        let vault = tempfile::tempdir().unwrap();
        data.save_current_vault(vault.path()).unwrap();
        assert_eq!(data.load_current_vault(), Some(vault.path().to_owned()));
    }

    #[test]
    fn last_note_round_trips_and_starts_unset() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().to_owned());
        let vault = tempfile::tempdir().unwrap();
        assert_eq!(data.load_last_note(vault.path()), None);

        let note = vault.path().join("my-note.tmt");
        data.save_last_note(vault.path(), &note).unwrap();
        assert_eq!(data.load_last_note(vault.path()), Some(note));
    }

    #[test]
    fn last_note_can_be_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().to_owned());
        let vault = tempfile::tempdir().unwrap();
        let note = vault.path().join("my-note.tmt");
        data.save_last_note(vault.path(), &note).unwrap();
        assert_eq!(data.load_last_note(vault.path()), Some(note));

        data.clear_last_note(vault.path());
        assert_eq!(data.load_last_note(vault.path()), None);
    }

    #[test]
    fn font_size_and_theme_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().to_owned());
        assert_eq!(data.load_font_size(), None);
        assert_eq!(data.load_theme(), None);

        data.save_font_size(2).unwrap();
        data.save_theme(1).unwrap();
        assert_eq!(data.load_font_size(), Some(2));
        assert_eq!(data.load_theme(), Some(1));
    }

    #[test]
    fn status_bar_choice_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().to_owned());
        assert_eq!(data.load_status_bar_choice(), None);

        data.save_status_bar_choice(1).unwrap();
        assert_eq!(data.load_status_bar_choice(), Some(1));
    }

    #[test]
    fn bottom_bar_config_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().to_owned());
        assert_eq!(data.load_bottom_bar_config(), None);

        data.save_bottom_bar_config("home,search,library,daily,quick")
            .unwrap();
        assert_eq!(
            data.load_bottom_bar_config(),
            Some("home,search,library,daily,quick".to_string())
        );
    }

    #[test]
    fn home_quick_actions_config_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().to_owned());
        assert_eq!(data.load_home_quick_actions_config(), None);

        data.save_home_quick_actions_config("new_note,search,daily")
            .unwrap();
        assert_eq!(
            data.load_home_quick_actions_config(),
            Some("new_note,search,daily".to_string())
        );
    }

    #[test]
    fn in_tree_gitdir_is_used_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().join("appdata"));
        let vault = tempfile::tempdir().unwrap();

        // Without .git, gitdir points inside appdata.
        let default_gitdir = data.gitdir(vault.path()).unwrap();
        assert!(default_gitdir.starts_with(dir.path().join("appdata")));

        // With in-tree .git, gitdir points to vault's .git.
        let in_tree_git = vault.path().join(".git");
        std::fs::create_dir(&in_tree_git).unwrap();
        assert_eq!(data.gitdir(vault.path()).unwrap(), in_tree_git);
    }

    #[test]
    fn in_tree_remote_is_detected_and_updated() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().join("appdata"));
        let vault = tempfile::tempdir().unwrap();

        // Initialize a standard git repo inside vault
        let repo = git2::Repository::init(vault.path()).unwrap();
        repo.remote("origin", "https://github.com/example/notes.git")
            .unwrap();

        // Automatically detected without remote.txt
        assert_eq!(
            data.load_remote(vault.path()),
            Some("https://github.com/example/notes.git".to_string())
        );

        // Updating via save_remote updates both remote.txt and .git config
        data.save_remote(vault.path(), "https://github.com/example/updated.git")
            .unwrap();
        assert_eq!(
            data.load_remote(vault.path()),
            Some("https://github.com/example/updated.git".to_string())
        );
        let updated_repo = git2::Repository::open(vault.path()).unwrap();
        let remote = updated_repo.find_remote("origin").unwrap();
        assert_eq!(remote.url(), Some("https://github.com/example/updated.git"));
    }

    #[test]
    fn paths_that_differ_only_by_a_separator_do_not_share_private_data() {
        let data = AppData::new(PathBuf::from("/appdata"));
        // Neither folder exists: the key must not depend on them existing.
        let underscore = data.vault_data_dir(Path::new("/srv/a_b"));
        let nested = data.vault_data_dir(Path::new("/srv/a/b"));
        assert_ne!(underscore, nested);
        // Still readable at a glance, and a bounded length however deep the path.
        assert!(underscore.ends_with(format!("a_b-{}", key_hash("/srv/a_b"))));
        let deep = "/x".repeat(500);
        assert!(
            data.vault_data_dir(Path::new(&deep))
                .file_name()
                .unwrap()
                .len()
                < 60
        );
    }

    fn key_hash(path: &str) -> String {
        let key = AppData::vault_key(Path::new(path));
        key.rsplit('-').next().unwrap().to_owned()
    }

    #[test]
    fn a_vault_whose_folder_went_missing_keeps_its_private_data_and_it_can_be_wiped() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().join("appdata"));
        let vault = dir.path().join("drive/notes");
        std::fs::create_dir_all(&vault).unwrap();
        data.add_vault(&vault).unwrap();
        data.save_remote(&vault, "https://example.invalid/notes.git")
            .unwrap();
        let registered = data.known_vaults().remove(0);
        let data_dir = data.vault_data_dir(&registered);
        assert!(data_dir.exists());

        // The drive is unmounted: the folder is gone.
        std::fs::remove_dir_all(dir.path().join("drive")).unwrap();
        assert_eq!(
            data.vault_data_dir(&registered),
            data_dir,
            "same key without the folder"
        );
        assert_eq!(
            data.load_remote(&registered).as_deref(),
            Some("https://example.invalid/notes.git")
        );

        data.remove_vault(&registered).unwrap();
        assert!(
            !data_dir.exists(),
            "its private data is wiped even though the folder is gone"
        );
    }

    #[test]
    fn removing_a_missing_vault_removes_only_that_vault() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().join("appdata"));
        // Two registered vaults whose folders are both gone.
        std::fs::write(
            dir.path().join("appdata/vaults.txt"),
            "/gone/one\n/gone/two\n",
        )
        .ok();
        std::fs::create_dir_all(dir.path().join("appdata")).unwrap();
        std::fs::write(
            dir.path().join("appdata/vaults.txt"),
            "/gone/one\n/gone/two",
        )
        .unwrap();
        data.remove_vault(Path::new("/gone/one")).unwrap();
        assert_eq!(data.known_vaults(), vec![PathBuf::from("/gone/two")]);
    }

    #[cfg(unix)]
    #[test]
    fn one_folder_reached_two_ways_is_one_vault() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().join("appdata"));
        let vault = dir.path().join("notes");
        std::fs::create_dir_all(&vault).unwrap();
        let alias = dir.path().join("alias");
        std::os::unix::fs::symlink(&vault, &alias).unwrap();

        data.add_vault(&vault).unwrap();
        data.add_vault(&alias).unwrap();
        data.add_vault(&dir.path().join("notes/../notes")).unwrap();
        assert_eq!(data.known_vaults(), vec![vault.canonicalize().unwrap()]);
        assert_eq!(data.vault_data_dir(&vault), data.vault_data_dir(&alias));
    }
}
