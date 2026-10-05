//! This device's own identity, for attributing *this device's* git
//! commits -- so a conflict's two sides can be labeled "Mine"/"Theirs"
//! honestly instead of guessing, without needing a stored mine/theirs
//! flag in the note itself (which would be backwards the moment a second
//! device looks at it -- see `immermemo`'s own
//! `.agents/tasks/git-ancestry-conflict-labeling.md` for the full
//! reasoning behind this crate existing at all).
//!
//! Two layers, deliberately kept separate:
//! - [`DeviceIdentity::device_id`]: a random, non-identifying string,
//!   generated once per install and never user-editable. This is the
//!   thing actually compared when deciding "did this device make this
//!   commit" -- embedded in a git commit's author *email*. Never derived
//!   from a hostname, username, or anything else that could identify the
//!   person behind the device.
//! - [`DeviceIdentity::display_name`]: optional, user-editable, purely
//!   cosmetic -- shown in the UI and, if set, in the git author *name*
//!   field too. Renaming this is always safe: it never touches
//!   `device_id`, so it can never change which commits this device
//!   recognizes as its own.
//!
//! This crate knows nothing about git (no `git2` dependency) or about
//! `immermemo-vault`'s `AppData` (no dependency on it either) -- it only
//! reads and writes a single small file, at a path the caller decides
//! (the same separation `immermemo-index`'s `NoteIndex::open(db_path)`
//! already has from `AppData::index_path`). `immermemo-sync` is the one
//! place that turns a [`DeviceIdentity`] into a `git2::Signature`.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// This device's own git identity. See the module doc for why
/// `device_id`/`display_name` are two separate things, not one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceIdentity {
    pub device_id: String,
    pub display_name: Option<String>,
}

/// Hex characters in a generated `device_id` -- 16 hex chars is 64 bits
/// of entropy, far more than this app's actual use case (telling apart a
/// handful of one person's own devices) ever needs, but cheap to afford.
const DEVICE_ID_LEN: usize = 16;

/// A process-lifetime counter, mixed into [`generate_device_id`] so two
/// calls in the same process (tests, mainly -- real usage only ever
/// generates one per install) can't collide even if the clock doesn't
/// advance between them.
static GENERATION_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh, random-enough, non-identifying device id. Not
/// cryptographically secure (no `rand`/`getrandom` dependency pulled in
/// for this -- see the module doc's "no dependency" stance): mixes wall-clock
/// time, this process's id, and a per-process counter through
/// [`std::collections::hash_map::DefaultHasher`] (SipHash-based) for
/// enough spread to make two devices colliding astronomically unlikely,
/// which is all this needs -- nothing here is a security boundary, see
/// the module doc.
fn generate_device_id() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let pid = std::process::id();
    let count = GENERATION_COUNTER.fetch_add(1, Ordering::Relaxed);

    let mut hasher = DefaultHasher::new();
    nanos.hash(&mut hasher);
    pid.hash(&mut hasher);
    count.hash(&mut hasher);
    let a = hasher.finish();

    // A second, differently-seeded hash to widen 64 bits of `DefaultHasher`
    // output into the full `DEVICE_ID_LEN` hex chars (8 bytes) asked for.
    let mut hasher2 = DefaultHasher::new();
    a.hash(&mut hasher2);
    count.hash(&mut hasher2);
    let b = hasher2.finish();

    let combined = format!("{a:016x}{b:016x}");
    combined[..DEVICE_ID_LEN].to_owned()
}

/// A user-supplied `display_name` can't contain a control character or
/// newline -- it lands in a plain-text file (one line per field) and,
/// downstream, a `git2::Signature`, neither of which tolerates one.
fn validate_display_name(name: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !name.is_empty(),
        "display name can't be empty -- clear it with None instead"
    );
    anyhow::ensure!(
        !name.chars().any(|c| c.is_control()),
        "display name can't contain control characters or newlines"
    );
    Ok(())
}

impl DeviceIdentity {
    /// Loads this device's identity from `path`, generating (and
    /// persisting) a fresh one on first run -- the only place
    /// [`generate_device_id`] is ever called from in real usage. Every
    /// later call reads the same `device_id` back.
    pub fn load_or_init(path: &Path) -> anyhow::Result<Self> {
        match Self::load(path)? {
            Some(identity) => Ok(identity),
            None => {
                let identity = DeviceIdentity {
                    device_id: generate_device_id(),
                    display_name: None,
                };
                identity.save(path)?;
                Ok(identity)
            }
        }
    }

    /// `None` if `path` doesn't exist yet (first run). `pub(crate)`-level
    /// detail kept private: callers almost always want
    /// [`load_or_init`](Self::load_or_init) instead, which never needs to
    /// distinguish "no file yet" from "a real identity" itself.
    fn load(path: &Path) -> anyhow::Result<Option<Self>> {
        let Ok(contents) = std::fs::read_to_string(path) else {
            return Ok(None);
        };
        let mut lines = contents.lines();
        let Some(device_id) = lines.next().map(str::to_owned).filter(|s| !s.is_empty()) else {
            return Ok(None);
        };
        let display_name = lines.next().map(str::to_owned).filter(|s| !s.is_empty());
        Ok(Some(DeviceIdentity {
            device_id,
            display_name,
        }))
    }

    /// Overwrites `path` wholesale with this identity's current state --
    /// same "small enough to rewrite, not worth a diff" convention
    /// `immermemo-vault`'s `appdata` module already uses for its own
    /// single-value files.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = format!(
            "{}\n{}\n",
            self.device_id,
            self.display_name.as_deref().unwrap_or("")
        );
        std::fs::write(path, contents)?;
        Ok(())
    }

    /// Validates and sets `display_name`, saving the result to `path`
    /// immediately -- there is no in-memory-only state for this crate's
    /// one caller to accidentally forget to persist. `None` clears it
    /// back to the default (unset).
    pub fn set_display_name(
        &mut self,
        path: &Path,
        display_name: Option<String>,
    ) -> anyhow::Result<()> {
        if let Some(name) = &display_name {
            validate_display_name(name)?;
        }
        self.display_name = display_name;
        self.save(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_load_generates_and_persists_a_device_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity");
        assert!(!path.exists());

        let identity = DeviceIdentity::load_or_init(&path).unwrap();
        assert!(!identity.device_id.is_empty());
        assert_eq!(identity.display_name, None);
        assert!(path.exists());

        // A second load reads the same id back, not a freshly generated one.
        let reloaded = DeviceIdentity::load_or_init(&path).unwrap();
        assert_eq!(reloaded.device_id, identity.device_id);
    }

    #[test]
    fn two_fresh_installs_never_collide_in_practice() {
        let dir = tempfile::tempdir().unwrap();
        let a = DeviceIdentity::load_or_init(&dir.path().join("a")).unwrap();
        let b = DeviceIdentity::load_or_init(&dir.path().join("b")).unwrap();
        assert_ne!(a.device_id, b.device_id);
    }

    #[test]
    fn display_name_round_trips_and_can_be_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity");
        let mut identity = DeviceIdentity::load_or_init(&path).unwrap();

        identity
            .set_display_name(&path, Some("Kaku's Phone".to_owned()))
            .unwrap();
        assert_eq!(
            DeviceIdentity::load_or_init(&path).unwrap().display_name,
            Some("Kaku's Phone".to_owned())
        );

        identity.set_display_name(&path, None).unwrap();
        assert_eq!(
            DeviceIdentity::load_or_init(&path).unwrap().display_name,
            None
        );
    }

    #[test]
    fn renaming_never_touches_the_device_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity");
        let mut identity = DeviceIdentity::load_or_init(&path).unwrap();
        let original_id = identity.device_id.clone();

        identity
            .set_display_name(&path, Some("New Name".to_owned()))
            .unwrap();
        assert_eq!(identity.device_id, original_id);
    }

    #[test]
    fn a_control_character_or_newline_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity");
        let mut identity = DeviceIdentity::load_or_init(&path).unwrap();

        assert!(
            identity
                .set_display_name(&path, Some("line one\nline two".to_owned()))
                .is_err()
        );
        assert!(
            identity
                .set_display_name(&path, Some("tab\tchar".to_owned()))
                .is_err()
        );
    }

    #[test]
    fn an_empty_display_name_is_rejected_use_none_instead() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity");
        let mut identity = DeviceIdentity::load_or_init(&path).unwrap();

        assert!(
            identity
                .set_display_name(&path, Some(String::new()))
                .is_err()
        );
    }
}
