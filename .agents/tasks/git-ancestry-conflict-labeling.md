${macro.generated\_by(self.path)}

# Per-device git identity, label conflicts by commit authorship

**Done, implemented and tested** (2026-10-05). Supersedes an earlier
version of this same file (see git history if that reasoning is wanted
again): this plan started as "record mine/theirs locally at merge time"
and was replaced, mid-design, once a better fix for the root cause
turned up. See "Done" below for what actually landed, in place of the
"Rough step shape" sketch that preceded it.

## Why

`.agents/tasks/adopt-std-conflict.md`'s "Git-ancestry mine/theirs
labeling" sections (not implemented) already settled that
`@conflict(a: ..., b: ...)` must stay neutral by design (`a`/`b` carry no
mine/theirs meaning -- see that file for why a *stored* mine/theirs label
is an actual correctness bug on a second device), but the UI still wants
to say "Mine"/"Theirs" when it can say so honestly.

**The actual root cause, found while scoping this task**:
`crates/sync/src/lib.rs`'s `Vault::commit_tree` signs every commit with
the exact same static identity, on every device:
`Signature::now("immermemo", "immermemo@local")`. Nothing in the shared
git history -- which is the only thing every device actually sees the
same copy of -- distinguishes which device authored which commit. A
live ancestry walk at resolution time (the earlier "decided: option 2")
can't work around that: the only thing that *could* tell devices apart
after the fact is each device's own reflog, which is purely local and
never transferred between devices in the first place, so it was never
going to help a *different* device either -- the "works on any device"
framing had a reasoning gap. A superseded middle design (recording
"which side was mine" locally, once, right at merge time) worked around
the symptom instead of the cause, and only ever covered "the same device,
shortly after its own merge" -- a real but narrow scope.

**The actual fix, decided in conversation with the user**: give each
device its own stable, non-identifying git identity, and compare commit
*authorship* instead of trying to reconstruct "mine" from branch
ancestry or a side-channel record. This restores the original "any
device, at any time" goal for real, using only what's already shared
over git (no reflog, no local cache file, no staleness bookkeeping).

## Device identity, decided

Two layers, not one -- this is the one design fork actually discussed
with the user, and the reasoning matters for anyone revisiting it:

- **A stable, random, non-PII `device_id`** (e.g. a short hex/base32
  string), generated once per install, persisted, and **never
  user-editable**. This is the thing actually compared for "is this
  commit mine" -- embedded in the git author's `email` field (e.g.
  `<device_id>@immermemo.local`). Never derived from hostname, username,
  or anything else that could identify the person. No migration/export
  story: if a fresh install (no local data carried over) gets a new
  `device_id`, the only consequence is conflicts from before the reinstall
  show neutral labels instead of mine/theirs -- never a *wrong* label,
  so this is an acceptable, rare, honest degradation, not a bug worth
  solving with an import/export flow. An ordinary OS-level
  backup/restore (iCloud, Android backup) already carries the app's local
  data, `device_id` included, with zero extra design needed.
- **An optional, user-editable `display_name`**, purely cosmetic: shown
  in the UI, and -- if the user sets one -- in the git author's `name`
  field too (so someone who browses their own history on GitHub, like
  this user does, can see a human-readable label if they want one).
  Defaults to unset (author `name` stays the plain `"immermemo"` it is
  today -- no personal information leaks by default). Renaming is always
  safe: it never touches `device_id`, so it can never break a conflict
  label, and needs no warning copy about "losing" past attribution --
  that whole problem only existed in the single-layer design the user
  first sketched and these two chose not to build.

## Current state (read before touching anything)

- `crates/sync/src/reconcile.rs`'s `Vault::merge_histories` is the one
  place `immermemo_merge::merge` is reached (via the free function
  `merge_bytes`, called per changed path) and the one place the
  resulting two-parent merge commit is made:
  `self.commit_tree("merge", &tree, &[&local_commit, &remote_commit])`
  -- parent order is `[local, remote]`, and `markers.rs`/`merge.rs`'s own
  fixed convention is "`local` becomes `a`, `remote` becomes `b`", so
  `HEAD.parent(0)`'s author made `a`'s content, `HEAD.parent(1)`'s author
  made `b`'s, *for that specific merge*. This plan's whole simplification
  (see "Rough step shape" below) rests on this: no deep revwalk needed
  for the common case, just read the two parents of one commit.
- `crates/sync`'s `Cargo.toml` already depends on `immermemo-vault`
  (currently unused in `lib.rs` itself -- nothing today reaches
  `AppData` from there). `crates/editor`'s `Cargo.toml` also already
  depends on `immermemo-vault` *and* `immermemo-sync`.
  `crates/editor/src/conflicts.rs`'s functions take only
  `vault_dir: &Path` today, never a `Vault`/`Repository` handle -- doing
  an authorship check from there needs either opening a transient `Vault`
  on demand, or having the app layer hand one through (not decided,
  see below).
- `crates/vault/src/appdata/` is a directory module (`mod.rs`, `paths.rs`,
  `registry.rs`, `settings.rs`), each an `impl AppData` block split by
  concern. `registry.rs` already holds *app-level* (not per-vault) state
  (`known_vaults`, `current_vault`) -- the natural shape for where a
  device identity lives too, since it's a fact about the install, not
  about any one linked vault. The crate has no `serde`/`serde_json`/
  `rusqlite` dependency today (deliberately minimal: `anyhow`, `git2`),
  and every existing value is a single plain string -- a `device_id`/
  `display_name` pair fits that exact shape, no new dependency needed.
- `crates/index`'s `NoteIndex` (SQLite) is a *rebuildable* cache
  (`reconcile`/`ReconcileReport` semantics, scanning the filesystem and
  overwriting what it finds) -- never the right home for identity data,
  and moot now anyway: this design needs no persisted-at-merge-time
  record at all, see below.
- `crates/editor/src/conflicts.rs`'s `conflict_sheet_state` already has
  a doc comment on its `mine`/`theirs` fields pointing at this exact gap
  (`a` = mine assumed unconditionally, citing `adopt-std-conflict.md`).
- `immermemo/src/sync.rs`/`session.rs` (not read yet) are presumably
  where the app layer calls `Vault::sync`/constructs `Vault` -- read
  before wiring the identity through there.
- `immermemo/ui/sheets/conflict.slint` (not read yet) -- whether it can
  render a neutral (non-mine/theirs) label at all is still unknown; see
  "Not decided yet".

## Decided: a new crate, `immermemo-identity`

- Owns `DeviceIdentity { device_id: String, display_name: Option<String> }`,
  its generation (`device_id`, once), load/save against a plain `&Path`,
  and validation of a user-supplied `display_name` (at minimum: no
  control characters/newlines, since it lands in a `git2::Signature`).
- **No dependency on `immermemo-vault`**, following `immermemo-index`'s
  own precedent: this crate just takes a path; the caller (`AppData`)
  decides *where* that path is, same separation already established for
  `NoteIndex::open(db_path)`.
- **No dependency on `git2` either**: this crate knows nothing about git
  -- `crates/sync` is the one place that turns a `DeviceIdentity` into a
  `Signature` (`Signature::now(display_name.as_deref().unwrap_or("immermemo"),
  &format!("{device_id}@immermemo.local"))`). Keeps `immermemo-identity`
  reusable if something non-git ever wants "which device is this" later
  (a device list in settings, say), and keeps it trivially unit-testable
  without any repository fixtures.
- `AppData` (`immermemo-vault`) gains one new **app-level** (not
  per-vault) path getter, e.g. `identity_path(&self) -> PathBuf`, sibling
  to how `known_vaults`/`current_vault` are already app-level rather than
  per-vault.

## Done

Every step below landed; `cargo test --workspace` is green (152 tests,
via `nix develop`), including the two cross-device integration tests
that are this feature's real proof: the same merge commit, read by two
different devices (and by a third that never touched it), answers
"whose side is this" correctly for each of them -- see
`crates/sync/src/lib.rs`'s `conflict_authorship_is_correctly_relative_to_each_device`
and `crates/editor/src/conflicts.rs`'s
`conflict_sheet_state_labels_each_devices_own_side_correctly`.

0. `immermemo-identity` (new crate): `DeviceIdentity { device_id,
   display_name }`, `load_or_init`/`save`/`set_display_name`,
   `device_id` generated once from wall-clock time + pid + a process
   counter hashed through `DefaultHasher` (no new dependency --
   `rand`/`getrandom` weren't already direct dependencies anywhere in
   the workspace, and nothing here needs cryptographic randomness, just
   "two personal devices never coincidentally collide"). Tests: fresh
   generation + persistence, two installs never collide, rename never
   touches `device_id`, control characters/newlines/empty string
   rejected.
1. `AppData::identity_path()` (`crates/vault/src/appdata/identity.rs`,
   new submodule) -- app-level, sibling to `registry`'s
   `known_vaults`/`current_vault`.
2. `Vault::set_identity`/`Vault::signature` (`crates/sync`): the
   optional-setter shape, as sketched -- `self.identity: None` keeps the
   original generic `Signature::now("immermemo", "immermemo@local")`
   (confirmed `commit_tree` was its only call site); `Some(identity)`
   signs with `display_name.unwrap_or("immermemo")` as the author name
   and `{device_id}@immermemo.local` as the email.
3. `Vault::conflict_authorship` (`crates/sync`): the "immediate case"
   check, exactly as sketched -- `HEAD` must be a two-parent commit whose
   tree entry for the note matches `current_text` exactly, then
   `HEAD.parent(0)`/`HEAD.parent(1)`'s author email decides `a`/`b`.
   Returns a three-state `ConflictSide { Mine, NotMine, Unknown }` per
   side, not a `bool`/`Option<bool>` -- "I know this isn't mine" is a
   real, useful answer, distinct from "I have no basis to say either
   way" (no identity set at all short-circuits to `Unknown`/`Unknown`
   before even touching git, for the same reason). The deeper
   "conflict resolved several syncs later" walk remains unbuilt, as
   planned -- falls back to `Unknown` whenever the immediate check
   doesn't apply.
4. `conflict_sheet_state` (`crates/editor/src/conflicts.rs`): gained
   `app_data: &AppData, vault_dir: &Path`, opens its own transient
   read-only `Vault` (same pattern `note_history::load_note_history`
   already used), and calls `conflict_authorship`. `ConflictSheetState::
   Active`'s `mine`/`theirs: String` fields became `a`/`b: String` (the
   content, unchanged in meaning) plus `a_side`/`b_side: ConflictSide` --
   resolved by reading `immermemo/ui/sheets/conflict.slint`, which
   turned out to hard-assume "Mine"/"Theirs" copy and a "📱 Local"/"☁️
   Remote" framing throughout, with no neutral-label path at all (itself
   an instance of the exact bug this feature fixes, since that framing
   was shown unconditionally regardless of which device was looking).
   Fixed by parametrizing: `App` (`app.slint`), `ConflictSheet`
   (`sheets/conflict.slint`) and `PropertiesScreen`
   (`screens/properties.slint`) all gained `a`/`b`-named content
   properties plus Rust-computed `*-label`/`*-button-label` strings
   (`immermemo/src/session.rs`'s `conflict_side_labels`: `Mine` ->
   "📱 Your changes"/"Keep Mine", `NotMine` -> "☁️ Their changes"/"Keep
   Theirs", `Unknown` -> "Version A"/"Keep A" (positional, neutral) --
   Slint itself never branches on `ConflictSide`, same "Rust decides
   meaning, Slint just draws" convention `classify.rs`'s `REGISTRY`
   already uses elsewhere in this app). The "Keep Both" button's label
   changed from "(Local → Remote)" to the neutral "(A → B)" for the same
   reason.
5. `immermemo/src/sync.rs`'s `run()` gained an `identity: Option<
   DeviceIdentity>` parameter (plain data, loaded by the caller --
   consistent with how `gitdir`/credentials already work, per that
   module's own stance); `session.rs`'s `start_sync` loads it via
   `DeviceIdentity::load_or_init(&s.app_data.identity_path())` before
   spawning the sync thread (a load failure degrades to `None`, signing
   with the generic signature -- not worth failing the whole sync over).
6. **Not done**: no UI to view/edit `display_name` yet (`settings.slint`
   is the obvious home, not built). Not blocking -- the feature works
   fully with every device left on the unset default; this is purely
   the cosmetic personalization layer.
7. `cargo build && cargo test --workspace` green across every crate
   (confirmed via `nix develop`), including `examples/snap.rs` (its own
   `set_active_conflict_mine`/`_theirs` calls renamed to match step 4's
   property rename).

## Dropped (superseded by the device-identity design above)

The earlier version of this plan is fully superseded, not just revised:
no appdata flat-file "conflict label" store, no `merge_bytes`/
`commit_tree` return-type changes to plumb a `clean`/`Oid` through
`merge_histories`, no per-note staleness/hash bookkeeping. All of that
existed to work around not being able to tell devices apart; the
device-identity fix removes the need for a side-channel record
entirely.

## Remaining / explicitly deferred

- No UI to view/edit `display_name` yet (step 6 above) -- a settings
  row, presumably; not scoped in detail.
- The deeper "conflict resolved several syncs later" walk (step 3's
  explicit non-goal) -- still falls back to neutral labels in that case,
  correct but less informative than it could be.
- `resolve_conflict_step`'s per-conflict (not per-note) indexing once a
  note can have more than one conflict at a time (today's
  `ConflictItem.index` is positional, not a stable id) --
  `adopt-std-conflict.md` already flags this as unresolved and it's
  orthogonal to this task, not re-solved here.
- The UI has not been visually checked on a real screen yet (only
  `cargo test`/`cargo check`) -- worth a real run before calling the UI
  side fully done, same caveat `adopt-std-conflict.md` ended on.
