use super::*;
use crate::gc::{AUTO_GC_SAMPLE_SHARD, AUTO_GC_SHARD_THRESHOLD};
use crate::testing::{NoCredentials, shared_remote};
use tempfile::TempDir;

/// A vault plus its own temp directories, kept alive for the test's
/// duration (the `Vault` only borrows their paths).
struct TestVault {
    working_tree: TempDir,
    _gitdir: TempDir,
    vault: Vault,
}

fn open_vault() -> TestVault {
    let working_tree = TempDir::new().unwrap();
    let gitdir = TempDir::new().unwrap();
    let vault = Vault::open(working_tree.path(), gitdir.path()).unwrap();
    TestVault {
        working_tree,
        _gitdir: gitdir,
        vault,
    }
}

#[test]
fn gitignore_is_preserved_and_rules_are_respected() {
    let remote = shared_remote();

    let mut a = open_vault();
    a.vault.set_remote(remote.path().to_str().unwrap()).unwrap();

    // Write a .gitignore, a normal note, and an ignored file.
    write_note(&a, ".gitignore", "*.tmp\nignored_dir/\n");
    write_note(&a, "note.tmt", "Hello.\n");
    write_note(&a, "scratch.tmp", "Temporary content.\n");
    std::fs::create_dir_all(a.working_tree.path().join("ignored_dir")).unwrap();
    write_note(&a, "ignored_dir/sub.tmt", "Ignored note.\n");

    a.vault.sync(&NoCredentials).unwrap();

    // Verify with another client: .gitignore and note.tmt arrive, but not scratch.tmp or ignored_dir.
    let mut b = open_vault();
    b.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
    b.vault.sync(&NoCredentials).unwrap();

    assert_eq!(read_note(&b, ".gitignore"), "*.tmp\nignored_dir/\n");
    assert_eq!(read_note(&b, "note.tmt"), "Hello.\n");
    assert!(!b.working_tree.path().join("scratch.tmp").exists());
    assert!(!b.working_tree.path().join("ignored_dir").exists());

    // Perform another sync on client a without changes: .gitignore must NOT be deleted.
    a.vault.sync(&NoCredentials).unwrap();
    assert_eq!(read_note(&a, ".gitignore"), "*.tmp\nignored_dir/\n");
    // And local ignored files on client a must NOT have been removed by remove_unwanted.
    assert!(a.working_tree.path().join("scratch.tmp").exists());
    assert!(a.working_tree.path().join("ignored_dir/sub.tmt").exists());

    // B syncs again and verifies .gitignore is still present in the git tree.
    b.vault.sync(&NoCredentials).unwrap();
    assert_eq!(read_note(&b, ".gitignore"), "*.tmp\nignored_dir/\n");
}

fn write_note(vault: &TestVault, name: &str, content: &str) {
    let path = vault.working_tree.path().join(name);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&path, content).unwrap();
}

fn read_note(vault: &TestVault, name: &str) -> String {
    std::fs::read_to_string(vault.working_tree.path().join(name)).unwrap()
}

#[test]
fn push_then_pull_round_trip() {
    let remote = shared_remote();

    let mut a = open_vault();
    a.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
    write_note(&a, "note.tmt", "Hello.\n");
    a.vault.sync(&NoCredentials).unwrap();

    let mut b = open_vault();
    b.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
    b.vault.sync(&NoCredentials).unwrap();

    assert_eq!(read_note(&b, "note.tmt"), "Hello.\n");
}

#[test]
fn edits_to_different_notes_merge_without_a_conflict() {
    let remote = shared_remote();

    let mut a = open_vault();
    a.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
    write_note(&a, "one.tmt", "One.\n");
    write_note(&a, "two.tmt", "Two.\n");
    a.vault.sync(&NoCredentials).unwrap();

    let mut b = open_vault();
    b.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
    b.vault.sync(&NoCredentials).unwrap();

    write_note(&a, "one.tmt", "One changed by A.\n");
    a.vault.sync(&NoCredentials).unwrap();

    write_note(&b, "two.tmt", "Two changed by B.\n");
    let report = b.vault.sync(&NoCredentials).unwrap();

    assert!(report.notes_needing_resolution.is_empty());
    assert_eq!(read_note(&b, "one.tmt"), "One changed by A.\n");
    assert_eq!(read_note(&b, "two.tmt"), "Two changed by B.\n");

    // A's next sync should pick up B's merge too.
    let report = a.vault.sync(&NoCredentials).unwrap();
    assert!(report.notes_needing_resolution.is_empty());
    assert_eq!(read_note(&a, "two.tmt"), "Two changed by B.\n");
}

#[test]
fn editing_the_same_word_is_reported_as_needing_resolution() {
    let remote = shared_remote();

    let mut a = open_vault();
    a.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
    write_note(&a, "note.tmt", "Bring a laptop.\n");
    a.vault.sync(&NoCredentials).unwrap();

    let mut b = open_vault();
    b.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
    b.vault.sync(&NoCredentials).unwrap();

    write_note(&a, "note.tmt", "Bring a charger.\n");
    a.vault.sync(&NoCredentials).unwrap();

    write_note(&b, "note.tmt", "Bring a notebook.\n");
    let report = b.vault.sync(&NoCredentials).unwrap();

    assert_eq!(
        report.notes_needing_resolution,
        vec![PathBuf::from("note.tmt")]
    );
    let merged = read_note(&b, "note.tmt");
    assert!(merged.contains("@conflict("));
    assert!(merged.contains("charger"));
    assert!(merged.contains("notebook"));
}

fn identity(device_id: &str) -> immermemo_identity::DeviceIdentity {
    immermemo_identity::DeviceIdentity {
        device_id: device_id.to_owned(),
        display_name: None,
    }
}

/// The whole point of per-device identity: the same merge commit,
/// read by two different devices, answers "whose side is this"
/// correctly for *each* of them -- never a stored flag that reads
/// backwards on the device that didn't run the merge.
#[test]
fn conflict_authorship_is_correctly_relative_to_each_device() {
    let remote = shared_remote();

    let mut a = open_vault();
    a.vault.set_identity(Some(identity("device-a")));
    a.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
    write_note(&a, "note.tmt", "Bring a laptop.\n");
    a.vault.sync(&NoCredentials).unwrap();

    let mut b = open_vault();
    b.vault.set_identity(Some(identity("device-b")));
    b.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
    b.vault.sync(&NoCredentials).unwrap();

    write_note(&a, "note.tmt", "Bring a charger.\n");
    a.vault.sync(&NoCredentials).unwrap();

    write_note(&b, "note.tmt", "Bring a notebook.\n");
    b.vault.sync(&NoCredentials).unwrap();
    let merged = read_note(&b, "note.tmt");

    // B ran this merge: B's own edit became `a`, A's incoming edit
    // became `b` (merge.rs's own fixed convention).
    let from_b = b.vault.conflict_authorship(Path::new("note.tmt"), &merged);
    assert_eq!(from_b.a, ConflictSide::Mine);
    assert_eq!(from_b.b, ConflictSide::NotMine);

    // A's next sync fast-forwards onto that exact same merge commit
    // (it's a direct descendant of A's own tip) -- no new merge, just
    // adopting B's commit object as-is.
    a.vault.sync(&NoCredentials).unwrap();
    assert_eq!(read_note(&a, "note.tmt"), merged);

    // From A's side, the same commit, the same `a`/`b` -- but the
    // answer flips, correctly: `a` (B's edit) is not A's, `b` (A's
    // own edit) is.
    let from_a = a.vault.conflict_authorship(Path::new("note.tmt"), &merged);
    assert_eq!(from_a.a, ConflictSide::NotMine);
    assert_eq!(from_a.b, ConflictSide::Mine);
}

/// A `Vault` nobody ever calls `set_identity` on never claims a side
/// as its own -- the honest default, not a bug (see the `identity`
/// field's own doc).
#[test]
fn conflict_authorship_is_unknown_without_an_identity() {
    let remote = shared_remote();

    let mut a = open_vault();
    a.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
    write_note(&a, "note.tmt", "Bring a laptop.\n");
    a.vault.sync(&NoCredentials).unwrap();

    let mut b = open_vault();
    b.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
    b.vault.sync(&NoCredentials).unwrap();

    write_note(&a, "note.tmt", "Bring a charger.\n");
    a.vault.sync(&NoCredentials).unwrap();
    write_note(&b, "note.tmt", "Bring a notebook.\n");
    b.vault.sync(&NoCredentials).unwrap();
    let merged = read_note(&b, "note.tmt");

    let authorship = b.vault.conflict_authorship(Path::new("note.tmt"), &merged);
    assert_eq!(authorship.a, ConflictSide::Unknown);
    assert_eq!(authorship.b, ConflictSide::Unknown);
}

/// If anything changed the note since the conflict-producing merge
/// (another sync, an edit around the conflict), `HEAD`'s tree no
/// longer matches `current_text` exactly -- the immediate-case check
/// this method does (see its own doc) correctly refuses to guess
/// rather than labeling against the wrong commit.
#[test]
fn conflict_authorship_is_unknown_once_the_note_no_longer_matches_head() {
    let remote = shared_remote();

    let mut a = open_vault();
    a.vault.set_identity(Some(identity("device-a")));
    a.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
    write_note(&a, "note.tmt", "Bring a laptop.\n");
    a.vault.sync(&NoCredentials).unwrap();

    let mut b = open_vault();
    b.vault.set_identity(Some(identity("device-b")));
    b.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
    b.vault.sync(&NoCredentials).unwrap();

    write_note(&a, "note.tmt", "Bring a charger.\n");
    a.vault.sync(&NoCredentials).unwrap();
    write_note(&b, "note.tmt", "Bring a notebook.\n");
    b.vault.sync(&NoCredentials).unwrap();

    let authorship = b
        .vault
        .conflict_authorship(Path::new("note.tmt"), "something else entirely");
    assert_eq!(authorship.a, ConflictSide::Unknown);
    assert_eq!(authorship.b, ConflictSide::Unknown);
}

#[test]
fn unrelated_histories_merge_cleanly() {
    let remote = shared_remote();

    // Client A initializes vault, writes note_a.tmt, syncs to remote (creates root commit on remote).
    let mut a = open_vault();
    a.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
    write_note(&a, "note_a.tmt", "Note A from Client A.\n");
    a.vault.sync(&NoCredentials).unwrap();

    // Client B initializes vault independently, writes note_b.tmt (creates local root commit before syncing).
    let mut b = open_vault();
    b.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
    write_note(&b, "note_b.tmt", "Note B from Client B.\n");

    // Client B syncs with remote. Local and remote have unrelated histories (no merge base).
    // This should not fail with "no merge base found", but produce a merge commit.
    let report = b.vault.sync(&NoCredentials).unwrap();
    assert!(report.notes_needing_resolution.is_empty());
    assert_eq!(read_note(&b, "note_a.tmt"), "Note A from Client A.\n");
    assert_eq!(read_note(&b, "note_b.tmt"), "Note B from Client B.\n");

    // Client A syncs again and receives note_b.tmt via fast-forward.
    let report = a.vault.sync(&NoCredentials).unwrap();
    assert!(report.notes_needing_resolution.is_empty());
    assert_eq!(read_note(&a, "note_a.tmt"), "Note A from Client A.\n");
    assert_eq!(read_note(&a, "note_b.tmt"), "Note B from Client B.\n");
}

#[test]
fn in_tree_git_repository_syncs_and_interoperates_with_external_git() {
    let remote = shared_remote();

    // Device A: PC setup with an in-tree git repository (e.g. git clone or git init).
    let notes_dir = tempfile::tempdir().unwrap();
    let in_tree_repo = Repository::init(notes_dir.path()).unwrap();
    let in_tree_gitdir = notes_dir.path().join(".git");

    let mut a = Vault::open(notes_dir.path(), &in_tree_gitdir).unwrap();
    a.set_remote(remote.path().to_str().unwrap()).unwrap();

    // Write a note from PC
    std::fs::write(notes_dir.path().join("pc_note.tmt"), "Written on PC\n").unwrap();
    let report = a.sync(&NoCredentials).unwrap();
    assert!(report.notes_needing_resolution.is_empty());

    // Verify git commit is visible to the in-tree repo and external git commands
    let head = in_tree_repo.head().unwrap().peel_to_commit().unwrap();
    assert_eq!(head.summary(), Some("sync"));

    // Verify working tree is clean for external git tools
    let statuses = in_tree_repo.statuses(None).unwrap();
    assert!(
        statuses.is_empty(),
        "Working tree should be clean in in-tree repo"
    );

    // Device B: Mobile / cloud setup using private bare repo
    let mut b = open_vault();
    b.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
    let report = b.vault.sync(&NoCredentials).unwrap();
    assert!(report.notes_needing_resolution.is_empty());
    assert_eq!(read_note(&b, "pc_note.tmt"), "Written on PC\n");

    // Device B writes mobile_note.tmt and syncs
    write_note(&b, "mobile_note.tmt", "Written on Mobile\n");
    b.vault.sync(&NoCredentials).unwrap();

    // Device A syncs and receives mobile_note.tmt
    let report = a.sync(&NoCredentials).unwrap();
    assert!(report.notes_needing_resolution.is_empty());
    assert_eq!(
        std::fs::read_to_string(notes_dir.path().join("mobile_note.tmt")).unwrap(),
        "Written on Mobile\n"
    );

    // In-tree repo continues to see working tree as clean
    let statuses = in_tree_repo.statuses(None).unwrap();
    assert!(
        statuses.is_empty(),
        "Working tree should remain clean after receiving remote changes"
    );
}

#[test]
fn unchanged_notes_preserve_mtime_across_sync() {
    let remote = shared_remote();

    let mut a = open_vault();
    a.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
    write_note(&a, "note_a.tmt", "Note A initial.\n");
    let report = a.vault.sync(&NoCredentials).unwrap();
    // Client A already has note_a.tmt on disk; local push requires no checkout disk writes.
    assert!(report.updated_notes.is_empty());

    let note_a_path = a.working_tree.path().join("note_a.tmt");
    let initial_mtime = std::fs::metadata(&note_a_path).unwrap().modified().unwrap();

    // Client B syncs and receives note_a.tmt as an incoming update
    let mut b = open_vault();
    b.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
    let report_b = b.vault.sync(&NoCredentials).unwrap();
    assert_eq!(report_b.updated_notes, vec![PathBuf::from("note_a.tmt")]);

    // Sleep briefly to ensure filesystem timestamp ticks
    std::thread::sleep(std::time::Duration::from_millis(50));

    // Client B adds note_b.tmt and syncs
    write_note(&b, "note_b.tmt", "Note B from B.\n");
    b.vault.sync(&NoCredentials).unwrap();

    // Client A syncs: receives note_b.tmt, note_a.tmt is unmodified
    let report = a.vault.sync(&NoCredentials).unwrap();
    assert_eq!(report.updated_notes, vec![PathBuf::from("note_b.tmt")]);
    assert!(report.deleted_notes.is_empty());

    // Note A must preserve its initial mtime (differential checkout did not touch it)
    let current_mtime = std::fs::metadata(&note_a_path).unwrap().modified().unwrap();
    assert_eq!(
        initial_mtime, current_mtime,
        "Untouched note mtime should be preserved"
    );

    // Sync again with no changes on either side
    let report = a.vault.sync(&NoCredentials).unwrap();
    assert!(report.updated_notes.is_empty());
    assert!(report.deleted_notes.is_empty());
}

#[test]
fn deleted_remote_notes_are_removed_and_reported() {
    let remote = shared_remote();

    let mut a = open_vault();
    a.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
    write_note(&a, "folder/nested.tmt", "Nested note content.\n");
    a.vault.sync(&NoCredentials).unwrap();

    let mut b = open_vault();
    b.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
    let report = b.vault.sync(&NoCredentials).unwrap();
    assert_eq!(
        report.updated_notes,
        vec![PathBuf::from("folder/nested.tmt")]
    );
    assert!(b.working_tree.path().join("folder/nested.tmt").exists());

    // Client A deletes nested.tmt and syncs
    std::fs::remove_file(a.working_tree.path().join("folder/nested.tmt")).unwrap();
    a.vault.sync(&NoCredentials).unwrap();

    // Client B syncs and receives the deletion
    let report = b.vault.sync(&NoCredentials).unwrap();
    assert_eq!(
        report.deleted_notes,
        vec![PathBuf::from("folder/nested.tmt")]
    );
    assert!(!b.working_tree.path().join("folder/nested.tmt").exists());
    assert!(
        !b.working_tree.path().join("folder").exists(),
        "Empty directory should be removed"
    );
}

#[test]
fn gc_packs_loose_objects_and_preserves_repository_integrity() {
    let remote = shared_remote();
    let mut a = open_vault();
    a.vault.set_remote(remote.path().to_str().unwrap()).unwrap();

    // Write a note and sync to create loose objects
    write_note(&a, "test.tmt", "Pack test content\n");
    a.vault.sync(&NoCredentials).unwrap();

    let objects_dir = a.vault.repo.path().join("objects");
    // Count loose object files before GC
    let mut loose_count_before = 0;
    for entry in std::fs::read_dir(&objects_dir).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.len() == 2
            && name.chars().all(|c| c.is_ascii_hexdigit())
            && let Ok(shard_entries) = std::fs::read_dir(entry.path())
        {
            loose_count_before += shard_entries.flatten().count();
        }
    }
    assert!(
        loose_count_before > 0,
        "Should have loose objects before GC"
    );

    // Run GC explicitly
    let gc_report = a.vault.gc().unwrap();
    assert!(gc_report.packed_objects > 0);
    assert_eq!(gc_report.pruned_objects, loose_count_before);

    // Pack directory should have .pack and .idx
    let pack_dir = objects_dir.join("pack");
    assert!(pack_dir.exists());
    let pack_files: Vec<_> = std::fs::read_dir(&pack_dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(pack_files.iter().any(|f| f.ends_with(".pack")));
    assert!(pack_files.iter().any(|f| f.ends_with(".idx")));

    // All loose shards should be gone
    let mut loose_count_after = 0;
    for entry in std::fs::read_dir(&objects_dir).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.len() == 2
            && name.chars().all(|c| c.is_ascii_hexdigit())
            && let Ok(shard_entries) = std::fs::read_dir(entry.path())
        {
            loose_count_after += shard_entries.flatten().count();
        }
    }
    assert_eq!(loose_count_after, 0, "All loose objects should be pruned");

    // Existing notes should be read cleanly from the packfile
    assert_eq!(read_note(&a, "test.tmt"), "Pack test content\n");

    // Subsequent commits and syncs continue to work seamlessly
    write_note(&a, "another.tmt", "Another note after GC\n");
    let report = a.vault.sync(&NoCredentials).unwrap();
    assert!(report.notes_needing_resolution.is_empty());
    assert_eq!(read_note(&a, "another.tmt"), "Another note after GC\n");
}

#[test]
fn should_auto_gc_detects_shard_threshold() {
    let a = open_vault();
    let sample_shard = a
        .vault
        .repo
        .path()
        .join("objects")
        .join(AUTO_GC_SAMPLE_SHARD);
    std::fs::create_dir_all(&sample_shard).unwrap();

    // Below threshold
    assert!(!a.vault.should_auto_gc());

    // Create dummy loose object files in the sample shard to reach threshold
    for i in 0..AUTO_GC_SHARD_THRESHOLD {
        std::fs::write(sample_shard.join(format!("dummy_{i}")), b"dummy").unwrap();
    }
    assert!(
        a.vault.should_auto_gc(),
        "Should trigger auto-GC when threshold is met"
    );

    // Clean up dummy files
    for i in 0..AUTO_GC_SHARD_THRESHOLD {
        let _ = std::fs::remove_file(sample_shard.join(format!("dummy_{i}")));
    }
    assert!(
        !a.vault.should_auto_gc(),
        "Should not trigger after cleanup"
    );
}

#[test]
fn auto_gc_runs_automatically_during_sync_when_threshold_met() {
    let remote = shared_remote();
    let mut a = open_vault();
    a.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
    write_note(&a, "note.tmt", "Initial content\n");

    // Populate sample shard to force auto-GC trigger on next sync
    let sample_shard = a
        .vault
        .repo
        .path()
        .join("objects")
        .join(AUTO_GC_SAMPLE_SHARD);
    std::fs::create_dir_all(&sample_shard).unwrap();
    for i in 0..AUTO_GC_SHARD_THRESHOLD {
        std::fs::write(sample_shard.join(format!("dummy_{i}")), b"dummy").unwrap();
    }
    assert!(a.vault.should_auto_gc());

    // Sync triggers auto-GC automatically
    let report = a.vault.sync(&NoCredentials).unwrap();
    assert!(report.notes_needing_resolution.is_empty());

    // Auto-GC should have run, creating pack and clearing loose objects
    let pack_dir = a.vault.repo.path().join("objects").join("pack");
    assert!(pack_dir.exists());
    assert!(
        !a.vault.should_auto_gc(),
        "Auto-GC should reset loose object condition"
    );
}

#[test]
fn file_history_and_vault_history_track_revisions_and_content() {
    let remote = shared_remote();
    let mut a = open_vault();
    a.vault.set_remote(remote.path().to_str().unwrap()).unwrap();

    // Commit 1: create note1 and note2
    write_note(&a, "note1.tmt", "Version 1 of note 1\n");
    write_note(&a, "note2.tmt", "Version 1 of note 2\n");
    a.vault.sync(&NoCredentials).unwrap();

    // Commit 2: modify note1 only
    write_note(&a, "note1.tmt", "Version 2 of note 1\n");
    a.vault.sync(&NoCredentials).unwrap();

    // Commit 3: modify note2 only
    write_note(&a, "note2.tmt", "Version 2 of note 2\n");
    a.vault.sync(&NoCredentials).unwrap();

    // Check file_history for note1.tmt (should only have 2 revisions: V2 and V1, newest first)
    let hist1 = a.vault.file_history(Path::new("note1.tmt"), 10).unwrap();
    assert_eq!(hist1.len(), 2);
    assert_eq!(hist1[0].content, "Version 2 of note 1\n");
    assert_eq!(hist1[1].content, "Version 1 of note 1\n");
    assert_eq!(hist1[0].short_id.len(), 7);

    // Check file_history for note2.tmt
    let hist2 = a.vault.file_history(Path::new("note2.tmt"), 10).unwrap();
    assert_eq!(hist2.len(), 2);
    assert_eq!(hist2[0].content, "Version 2 of note 2\n");
    assert_eq!(hist2[1].content, "Version 1 of note 2\n");

    // Check vault_history (total 3 commits)
    let vault_hist = a.vault.vault_history(10).unwrap();
    assert_eq!(vault_hist.len(), 3);
}

#[test]
fn empty_working_tree_with_existing_head_does_not_wipe_out_notes() {
    let remote = shared_remote();

    let mut a = open_vault();
    a.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
    write_note(&a, "note1.tmt", "Content 1\n");
    write_note(&a, "note2.tmt", "Content 2\n");
    a.vault.sync(&NoCredentials).unwrap();

    // Simulate a scenario where a vault has an existing gitdir with history,
    // but its working tree is completely empty (e.g. unhydrated vault,
    // deleted local notes folder, or fresh link onto existing gitdir).
    let empty_wt = TempDir::new().unwrap();
    let mut b = Vault::open(empty_wt.path(), a._gitdir.path()).unwrap();
    b.set_remote(remote.path().to_str().unwrap()).unwrap();

    // Verify the working tree is empty before sync.
    assert_eq!(std::fs::read_dir(empty_wt.path()).unwrap().count(), 0);

    // Sync must NOT wipe out the remote! Instead, it must hydrate the files.
    let report = b.sync(&NoCredentials).unwrap();
    assert_eq!(report.deleted_notes.len(), 0);
    assert!(report.updated_notes.contains(&PathBuf::from("note1.tmt")));
    assert!(report.updated_notes.contains(&PathBuf::from("note2.tmt")));

    // Working tree should now have the files restored.
    assert_eq!(
        std::fs::read_to_string(empty_wt.path().join("note1.tmt")).unwrap(),
        "Content 1\n"
    );
    assert_eq!(
        std::fs::read_to_string(empty_wt.path().join("note2.tmt")).unwrap(),
        "Content 2\n"
    );

    // Verify remote repository still has all notes intact.
    let remote_repo = Repository::open_bare(remote.path()).unwrap();
    let remote_head = remote_repo
        .find_reference("refs/heads/main")
        .unwrap()
        .peel_to_commit()
        .unwrap();
    assert_eq!(remote_head.tree().unwrap().len(), 2);
}

#[test]
fn legitimate_single_note_deletion_is_still_committed() {
    let remote = shared_remote();

    let mut a = open_vault();
    a.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
    write_note(&a, "keep.tmt", "Keep me\n");
    write_note(&a, "delete.tmt", "Delete me\n");
    a.vault.sync(&NoCredentials).unwrap();

    // Client B syncs both notes
    let mut b = open_vault();
    b.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
    b.vault.sync(&NoCredentials).unwrap();
    assert_eq!(read_note(&b, "delete.tmt"), "Delete me\n");

    // Client A intentionally deletes one note, keeping the other
    std::fs::remove_file(a.working_tree.path().join("delete.tmt")).unwrap();
    a.vault.sync(&NoCredentials).unwrap();

    // Client B syncs and receives the deletion
    let report_b = b.vault.sync(&NoCredentials).unwrap();
    assert_eq!(report_b.deleted_notes, vec![PathBuf::from("delete.tmt")]);
    assert!(!b.working_tree.path().join("delete.tmt").exists());
    assert_eq!(read_note(&b, "keep.tmt"), "Keep me\n");

    // Remote also reflects this deletion
    let remote_repo = Repository::open_bare(remote.path()).unwrap();
    let remote_head = remote_repo
        .find_reference("refs/heads/main")
        .unwrap()
        .peel_to_commit()
        .unwrap();
    assert_eq!(remote_head.tree().unwrap().len(), 1);
    assert!(remote_head.tree().unwrap().get_name("keep.tmt").is_some());
    assert!(remote_head.tree().unwrap().get_name("delete.tmt").is_none());
}
