use std::path::{Path, PathBuf};

use immermemo_index::NoteIndex;
use immermemo_merge::ConflictResolution;
use immermemo_sync::{ConflictSide, Vault};
use immermemo_vault::appdata::AppData;

use crate::state::{EditorState, current_path};

pub struct ConflictResolved {
    pub body: String,
    /// Whether the note still has unresolved conflicts after this step.
    pub has_conflict: bool,
    pub status: String,
}

/// Resolves every conflict in the current note at once, in favor of one
/// side. No UI currently calls this -- the editor/properties "Keep
/// Mine"/"Keep Theirs" bar resolves one conflict at a time via
/// [`resolve_conflict_step`] instead, so that a same-looking button means
/// the same thing on every screen. Kept around (and covered by this
/// crate's tests) for a future bulk-resolve feature.
#[allow(dead_code)]
pub fn resolve_active_conflict(
    state: &mut EditorState,
    vault_dir: &Path,
    notes: &[PathBuf],
    conflicted: &mut [bool],
    index: &mut NoteIndex,
    resolution: ConflictResolution,
) -> Option<Result<ConflictResolved, String>> {
    let path = current_path(state, notes)?;
    let current_text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => return Some(Err(format!("Could not read note: {e}"))),
    };
    let doc = match tomet_parser::parse_document(&current_text) {
        Ok(d) => d,
        Err(e) => {
            return Some(Err(format!(
                "Could not parse note for conflict resolution: {e}"
            )));
        }
    };
    let side_name = match &resolution {
        ConflictResolution::A => "local",
        ConflictResolution::B => "remote",
        _ => "custom",
    };
    let (resolved_text, _resolved_doc) = resolve_all_lossless(&current_text, &resolution)
        .unwrap_or_else(|| {
            let resolved_doc = immermemo_merge::resolve_all(&doc, resolution);
            let resolved_text = tomet_printer::document_to_tm(&resolved_doc);
            (resolved_text, resolved_doc)
        });

    if let Err(e) = std::fs::write(&path, &resolved_text) {
        return Some(Err(format!("Save failed: {e}")));
    }
    if let Some(history) = state.history.as_mut() {
        history.edit(&resolved_text);
    }
    if let Ok(rel) = path.strip_prefix(vault_dir) {
        let _ = index.record_write(vault_dir, rel, &resolved_text);
        let _ = index.set_conflict(rel, false);
    }
    if let Some(cur) = state.current
        && cur < conflicted.len()
    {
        conflicted[cur] = false;
    }

    Some(Ok(ConflictResolved {
        body: resolved_text,
        has_conflict: false,
        status: format!("Resolved conflict (kept {side_name} version)"),
    }))
}

/// Resolves only the conflict at `active_conflict_index`, one at a time --
/// the editor/properties quick-resolve bar and the full `ConflictSheet`
/// both walk conflicts this way, one step per tap.
pub fn resolve_conflict_step(
    state: &mut EditorState,
    vault_dir: &Path,
    notes: &[PathBuf],
    conflicted: &mut [bool],
    index: &mut NoteIndex,
    active_conflict_index: i32,
    choice: i32,
) -> Option<Result<ConflictResolved, String>> {
    let path = current_path(state, notes)?;
    let current_text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => return Some(Err(format!("Could not read note: {e}"))),
    };
    let doc = match tomet_parser::parse_document(&current_text) {
        Ok(d) => d,
        Err(e) => {
            return Some(Err(format!(
                "Could not parse note for conflict resolution: {e}"
            )));
        }
    };
    let target_idx = active_conflict_index.max(0) as usize;
    let resolution = match choice {
        0 => ConflictResolution::A,
        1 => ConflictResolution::B,
        _ => ConflictResolution::Both,
    };
    let choice_name = match choice {
        0 => "kept local",
        1 => "kept remote",
        _ => "kept both",
    };

    let (resolved_text, resolved_doc) =
        resolve_single_lossless(&current_text, target_idx, &resolution).unwrap_or_else(|| {
            let resolved_doc = immermemo_merge::resolve_single(&doc, target_idx, resolution);
            let resolved_text = tomet_printer::document_to_tm(&resolved_doc);
            (resolved_text, resolved_doc)
        });

    if let Err(e) = std::fs::write(&path, &resolved_text) {
        return Some(Err(format!("Save failed: {e}")));
    }
    if let Some(history) = state.history.as_mut() {
        history.edit(&resolved_text);
    }

    let remaining_conflicts = immermemo_merge::find_conflicts(&resolved_doc);
    let has_conflict = !remaining_conflicts.is_empty();
    if let Ok(rel) = path.strip_prefix(vault_dir) {
        let _ = index.record_write(vault_dir, rel, &resolved_text);
        let _ = index.set_conflict(rel, has_conflict);
    }
    if let Some(cur) = state.current
        && cur < conflicted.len()
    {
        conflicted[cur] = has_conflict;
    }

    let status = if has_conflict {
        format!("Resolved conflict ({choice_name})")
    } else {
        "All conflicts resolved".to_owned()
    };
    Some(Ok(ConflictResolved {
        body: resolved_text,
        has_conflict,
        status,
    }))
}

pub enum ConflictSheetState {
    /// No note is open, or its content couldn't be read/parsed -- nothing
    /// to show, and whether it "has a conflict" is left untouched.
    NoActiveNote,
    /// The open note parsed cleanly and has no conflicts.
    NoConflicts,
    Active {
        total: usize,
        index: usize,
        a: String,
        b: String,
        /// Whether `a`/`b` are *this* device's own content, per
        /// `immermemo_sync::Vault::conflict_authorship` -- `Unknown` for
        /// both whenever that can't be determined (no identity set, the
        /// note has changed since the conflict-producing merge, or this
        /// device never ran that merge at all). The caller decides what
        /// to show instead of a bare "Mine"/"Theirs" label in that case;
        /// this never guesses.
        a_side: ConflictSide,
        b_side: ConflictSide,
    },
}

/// Recomputes the `ConflictSheet`'s state for the currently open note,
/// clamping `requested_index` (the sheet's own `active-conflict-index`)
/// into range.
pub fn conflict_sheet_state(
    state: &EditorState,
    notes: &[PathBuf],
    requested_index: i32,
    app_data: &AppData,
    vault_dir: &Path,
) -> ConflictSheetState {
    let Some(path) = current_path(state, notes) else {
        return ConflictSheetState::NoActiveNote;
    };
    let Ok(current_text) = std::fs::read_to_string(&path) else {
        return ConflictSheetState::NoActiveNote;
    };
    let Ok(doc) = tomet_parser::parse_document(&current_text) else {
        return ConflictSheetState::NoActiveNote;
    };
    let items = immermemo_merge::find_conflicts(&doc);
    if items.is_empty() {
        return ConflictSheetState::NoConflicts;
    }
    let total = items.len();
    let index = requested_index.max(0).min(total as i32 - 1) as usize;
    let item = &items[index];

    // Every `@conflict` still in this note came from the same merge
    // commit (or none of them did, if something's changed since) -- one
    // authorship check covers the whole note, not one per conflict.
    let (a_side, b_side) = conflict_authorship(app_data, vault_dir, &path, &current_text);

    ConflictSheetState::Active {
        total,
        index,
        a: item.a.clone(),
        b: item.b.clone(),
        a_side,
        b_side,
    }
}

/// `(a_side, b_side)` for every `@conflict` in `path`'s current content
/// -- see `immermemo_sync::Vault::conflict_authorship`'s own doc. Opens
/// its own transient, read-only `Vault` (same pattern as
/// `note_history::load_note_history`) rather than needing a persistent
/// one threaded through from the caller. `Unknown`/`Unknown` if the
/// vault can't be opened at all, or `path` isn't inside `vault_dir`.
fn conflict_authorship(
    app_data: &AppData,
    vault_dir: &Path,
    path: &Path,
    current_text: &str,
) -> (ConflictSide, ConflictSide) {
    let unknown = (ConflictSide::Unknown, ConflictSide::Unknown);
    let Ok(rel_path) = path.strip_prefix(vault_dir) else {
        return unknown;
    };
    let Ok(gitdir) = app_data.gitdir(vault_dir) else {
        return unknown;
    };
    let Ok(mut vault) = Vault::open(vault_dir, &gitdir) else {
        return unknown;
    };
    vault.set_identity(
        immermemo_identity::DeviceIdentity::load_or_init(&app_data.identity_path()).ok(),
    );
    let authorship = vault.conflict_authorship(rel_path, current_text);
    (authorship.a, authorship.b)
}

fn resolve_single_lossless(
    src: &str,
    target_idx: usize,
    resolution: &ConflictResolution,
) -> Option<(String, tomet_ast::Document)> {
    use tomet_edit::{export_doc, import_source};

    let mut doc = import_source(src);
    let mut current_conflict_idx = 0;
    let mut resolved = false;

    let node_ids = collect_leaf_node_ids(&doc);

    for node_id in node_ids {
        let Some(node) = doc.get(node_id) else {
            continue;
        };
        if !node.raw.contains(immermemo_merge::CONFLICT_MARKER) {
            continue;
        }

        let Ok(parsed) = tomet_parser::parse_document(&node.raw) else {
            continue;
        };
        let node_conflicts = immermemo_merge::find_conflicts(&parsed);
        let count = node_conflicts.len();

        if count == 0 {
            continue;
        }

        if target_idx >= current_conflict_idx && target_idx < current_conflict_idx + count {
            let local_idx = target_idx - current_conflict_idx;
            let resolved_ast =
                immermemo_merge::resolve_single(&parsed, local_idx, resolution.clone());
            let mut new_text = tomet_printer::document_to_tm(&resolved_ast);

            if node.raw.ends_with('\n') && !new_text.ends_with('\n') {
                new_text.push('\n');
            } else if !node.raw.ends_with('\n') && new_text.ends_with('\n') {
                new_text.pop();
            }

            let node_mut = doc.get_mut(node_id).unwrap();
            node_mut.kind = tomet_edit::NodeKind::Paragraph {
                text: new_text.clone(),
            };
            node_mut.raw = new_text.clone();
            node_mut.mark_edited();
            resolved = true;
            break;
        }

        current_conflict_idx += count;
    }

    if !resolved {
        return None;
    }

    let resolved_text = export_doc(&doc);
    let resolved_doc = tomet_parser::parse_document(&resolved_text).ok()?;
    Some((resolved_text, resolved_doc))
}

fn resolve_all_lossless(
    src: &str,
    resolution: &ConflictResolution,
) -> Option<(String, tomet_ast::Document)> {
    use tomet_edit::{export_doc, import_source};

    let mut doc = import_source(src);
    let node_ids = collect_leaf_node_ids(&doc);
    let mut any_resolved = false;

    for node_id in node_ids {
        let Some(node) = doc.get(node_id) else {
            continue;
        };
        if !node.raw.contains(immermemo_merge::CONFLICT_MARKER) {
            continue;
        }

        let Ok(parsed) = tomet_parser::parse_document(&node.raw) else {
            continue;
        };
        if immermemo_merge::find_conflicts(&parsed).is_empty() {
            continue;
        }

        let resolved_ast = immermemo_merge::resolve_all(&parsed, resolution.clone());
        let mut new_text = tomet_printer::document_to_tm(&resolved_ast);
        if node.raw.ends_with('\n') && !new_text.ends_with('\n') {
            new_text.push('\n');
        } else if !node.raw.ends_with('\n') && new_text.ends_with('\n') {
            new_text.pop();
        }

        let node_mut = doc.get_mut(node_id).unwrap();
        node_mut.kind = tomet_edit::NodeKind::Paragraph {
            text: new_text.clone(),
        };
        node_mut.raw = new_text.clone();
        node_mut.mark_edited();
        any_resolved = true;
    }

    if !any_resolved {
        return None;
    }

    let resolved_text = export_doc(&doc);
    let resolved_doc = tomet_parser::parse_document(&resolved_text).ok()?;
    Some((resolved_text, resolved_doc))
}

fn collect_leaf_node_ids(doc: &tomet_edit::EditDoc) -> Vec<tomet_edit::NodeId> {
    let mut out = Vec::new();
    for item in doc.root_children() {
        if let Some(id) = item.as_node_id() {
            collect_nodes_recursive(doc, id, &mut out);
        }
    }
    out
}

fn collect_nodes_recursive(
    doc: &tomet_edit::EditDoc,
    id: tomet_edit::NodeId,
    out: &mut Vec<tomet_edit::NodeId>,
) {
    let Some(node) = doc.get(id) else {
        return;
    };
    match &node.kind {
        tomet_edit::NodeKind::Section { children, .. }
        | tomet_edit::NodeKind::List { children }
        | tomet_edit::NodeKind::ListItem { children, .. } => {
            for child in children {
                if let Some(cid) = child.as_node_id() {
                    collect_nodes_recursive(doc, cid, out);
                }
            }
        }
        _ => {
            out.push(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use immermemo_identity::DeviceIdentity;
    use tempfile::TempDir;

    struct NoCredentials;
    impl immermemo_sync::CredentialProvider for NoCredentials {
        fn credentials(&self, _remote_url: &str) -> anyhow::Result<git2::Cred> {
            anyhow::bail!("not expected to be called for a local-path transport")
        }
    }

    /// One "device": a vault working tree plus its own app-data base,
    /// signing every commit it makes with its own `device_id` -- same
    /// shape `immermemo_sync`'s own tests use, plus an `app_data` this
    /// crate's `conflict_sheet_state` can be driven through directly.
    struct Device {
        working_tree: TempDir,
        app_data: AppData,
        _app_data_dir: TempDir,
    }

    impl Device {
        fn new(device_id: &'static str) -> Self {
            let app_data_dir = TempDir::new().unwrap();
            let app_data = AppData::new(app_data_dir.path().to_owned());
            // Persisted up front, not just held in memory: `conflict_sheet_state`
            // reads this device's identity back from `app_data.identity_path()`
            // itself (via `DeviceIdentity::load_or_init`), same as the real
            // app does -- an in-memory-only identity here would make
            // `load_or_init` generate an unrelated random one instead of
            // this test's fixed `device_id`.
            DeviceIdentity {
                device_id: device_id.to_owned(),
                display_name: None,
            }
            .save(&app_data.identity_path())
            .unwrap();
            Self {
                working_tree: TempDir::new().unwrap(),
                app_data,
                _app_data_dir: app_data_dir,
            }
        }

        fn write(&self, name: &str, text: &str) {
            std::fs::write(self.working_tree.path().join(name), text).unwrap();
        }

        fn read(&self, name: &str) -> String {
            std::fs::read_to_string(self.working_tree.path().join(name)).unwrap()
        }

        fn sync(&self, remote: &Path) {
            let gitdir = self.app_data.gitdir(self.working_tree.path()).unwrap();
            let mut vault = Vault::open(self.working_tree.path(), &gitdir).unwrap();
            vault.set_identity(DeviceIdentity::load_or_init(&self.app_data.identity_path()).ok());
            vault.set_remote(remote.to_str().unwrap()).unwrap();
            vault.sync(&NoCredentials).unwrap();
        }

        fn state_for(&self, note: &str) -> (EditorState, Vec<PathBuf>) {
            let path = self.working_tree.path().join(note);
            (
                EditorState {
                    current: Some(0),
                    history: None,
                    note_history_revisions: Vec::new(),
                },
                vec![path],
            )
        }
    }

    fn shared_remote() -> TempDir {
        let dir = TempDir::new().unwrap();
        git2::Repository::init_bare(dir.path()).unwrap();
        dir
    }

    /// The seam this whole feature exists for: the same conflicted note,
    /// opened on each of the two devices that produced it, labels each
    /// device's own side correctly -- not from a stored flag, but from
    /// `conflict_sheet_state` wiring through to `Vault::conflict_authorship`
    /// with each device's own identity.
    #[test]
    fn conflict_sheet_state_labels_each_devices_own_side_correctly() {
        let remote = shared_remote();

        let a = Device::new("device-a");
        a.write("note.tmt", "Bring a laptop.\n");
        a.sync(remote.path());

        let b = Device::new("device-b");
        b.sync(remote.path());

        a.write("note.tmt", "Bring a charger.\n");
        a.sync(remote.path());
        b.write("note.tmt", "Bring a notebook.\n");
        b.sync(remote.path());

        let (state, notes) = b.state_for("note.tmt");
        let ConflictSheetState::Active { a_side, b_side, .. } =
            conflict_sheet_state(&state, &notes, 0, &b.app_data, b.working_tree.path())
        else {
            panic!("expected an active conflict");
        };
        assert_eq!(a_side, ConflictSide::Mine);
        assert_eq!(b_side, ConflictSide::NotMine);

        // A's next sync fast-forwards onto the same merge commit -- same
        // note content, opposite answer, correctly.
        a.sync(remote.path());
        assert_eq!(a.read("note.tmt"), b.read("note.tmt"));

        let (state, notes) = a.state_for("note.tmt");
        let ConflictSheetState::Active { a_side, b_side, .. } =
            conflict_sheet_state(&state, &notes, 0, &a.app_data, a.working_tree.path())
        else {
            panic!("expected an active conflict");
        };
        assert_eq!(a_side, ConflictSide::NotMine);
        assert_eq!(b_side, ConflictSide::Mine);
    }

    /// A *third* device -- one that never participated in the merge that
    /// produced this conflict, syncing in afterward with its own
    /// freshly-generated identity -- correctly sees neither side as its
    /// own, rather than defaulting to "mine" for either.
    #[test]
    fn conflict_sheet_state_shows_not_mine_for_a_third_devices_conflict() {
        let remote = shared_remote();

        let a = Device::new("device-a");
        a.write("note.tmt", "Bring a laptop.\n");
        a.sync(remote.path());

        let b = Device::new("device-b");
        b.sync(remote.path());

        a.write("note.tmt", "Bring a charger.\n");
        a.sync(remote.path());
        b.write("note.tmt", "Bring a notebook.\n");
        b.sync(remote.path());

        // A third device, synced in after the fact -- never ran this
        // merge, and `DeviceIdentity::load_or_init` gives it a fresh
        // identity with no relationship to either side.
        let c = Device::new("device-c");
        c.sync(remote.path());

        let (state, notes) = c.state_for("note.tmt");
        let ConflictSheetState::Active { a_side, b_side, .. } =
            conflict_sheet_state(&state, &notes, 0, &c.app_data, c.working_tree.path())
        else {
            panic!("expected an active conflict");
        };
        assert_eq!(a_side, ConflictSide::NotMine);
        assert_eq!(b_side, ConflictSide::NotMine);
    }

    #[test]
    fn resolve_single_preserves_comments_and_surrounding_layout() {
        let src = "// Top comment\n\n\nFirst paragraph.\n\n@conflict(\n  a: [ Mine. ]\n  b: [ Theirs. ]\n)\n\n\n// Bottom comment\n";
        let (resolved_text, doc) =
            resolve_single_lossless(src, 0, &ConflictResolution::A).unwrap();

        assert!(resolved_text.starts_with("// Top comment\n\n\nFirst paragraph.\n\n"));
        assert!(resolved_text.contains("Mine."));
        assert!(!resolved_text.contains("Theirs."));
        assert!(resolved_text.ends_with("\n\n\n// Bottom comment\n"));

        let remaining = immermemo_merge::find_conflicts(&doc);
        assert!(remaining.is_empty());
    }

    #[test]
    fn resolve_all_preserves_comments_and_surrounding_layout() {
        let src = "// Top comment\n\n\nParagraph 1.\n\n@conflict(\n  a: [ A1. ]\n  b: [ B1. ]\n)\n\nMiddle text.\n\n@conflict(\n  a: [ A2. ]\n  b: [ B2. ]\n)\n\n\n// Bottom comment\n";
        let (resolved_text, doc) =
            resolve_all_lossless(src, &ConflictResolution::B).unwrap();

        assert!(resolved_text.starts_with("// Top comment\n\n\nParagraph 1.\n\n"));
        assert!(resolved_text.contains("B1."));
        assert!(resolved_text.contains("B2."));
        assert!(!resolved_text.contains("A1."));
        assert!(!resolved_text.contains("A2."));
        assert!(resolved_text.ends_with("\n\n\n// Bottom comment\n"));

        let remaining = immermemo_merge::find_conflicts(&doc);
        assert!(remaining.is_empty());
    }
}
