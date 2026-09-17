# core/sync: implement Vault (bare repo + manual index/checkout)

Design reference: `core/sync/src/lib.rs` module doc, `docs/design.md`'s
six-phase sync description.

## Steps

1. [x] `Vault::open`: init a bare repo at `private_gitdir` if absent,
   open it otherwise. No workdir association in libgit2 at all.
2. [x] Internal staging (`commit_working_tree`/`stage_dir`/`stage_bytes`):
   reads every file under `working_tree` (dotfiles/dot-dirs skipped),
   stages each via `Index::add_frombuffer`, commits. Found and fixed a
   real bug during testing: committing unconditionally on every sync
   created a bogus empty root commit on a freshly linked vault (empty
   working tree, no local history yet), which then had no common
   ancestor with an already-populated remote and made `merge_base` fail
   outright. Fixed by skipping the commit when there is no parent *and*
   nothing staged.
3. [x] Internal checkout (`checkout_head`): walks the tree, writes blobs
   into `working_tree`, removes working-tree files the tree no longer
   has (`remove_unwanted`, diffing the wanted-path set rather than
   trusting a directory listing).
4. [x] `Vault::set_remote`.
5. [x] `Vault::sync`: commit → loop { fetch, reconcile (fast-forward /
   adopt / real merge via `merge_histories`), push; retry on rejection }
   → checkout. `merge_histories` walks the union of paths across
   base/local/remote trees and, per path, either takes the
   unchanged/added/deleted side automatically or (both sides changed a
   `.tmt` file differently) calls `immermemo_merge::merge`.
6. [x] Tests against a real local bare repo standing in for a hosted
   remote (`tempfile` + `Repository::init_bare`, added via `set_remote`
   with a plain filesystem path -- local transport, no network). Three
   scenarios: push-then-pull, two vaults editing different notes (merges
   silently), two vaults editing the same word (reported in
   `notes_needing_resolution`, contains the expected markers). 3/3
   passing, 19/19 across the whole workspace.

## Decisions made without asking first -- flagged for the app's author

Two policies got picked during implementation because *something* had to
happen, not because they were settled:

- **Non-`.tmt` files** (attachments) that changed differently on both
  sides just keep `local`'s bytes. No merge, no duplication, no LFS --
  `docs/design.md` already lists the attachment story as undecided.
- **Modify/delete conflicts** (one side edited a file, the other deleted
  it) resolve by keeping the edit. Chosen to avoid silently discarding
  someone's edit, not discussed.

Both are recorded in `core/sync/src/lib.rs`'s module doc ("What isn't
decided yet") so they don't read as settled just because there's code.

## Status

Steps 1-6 done and tested.
