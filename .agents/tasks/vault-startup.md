# Vault startup, registry and failure visibility

Requested by the user ("the path argument is legacy: the settings already
keep the vault list"). All four items chosen; `immermemo <dir>` registers the
vault and switches to it. Backward compatibility is explicitly NOT a goal
(the user's words), so no migrations.

Based on the uncommitted `lfs-images` change (D touches its media code).

## Findings (evidence)

- `app/src/main.rs` bails with `usage: immermemo <notes-dir>` with no arg, yet
  `Session::start` only uses the dir when no vault is known: otherwise it is
  silently ignored.
- `Session::start` `create_dir_all`s the current vault and builds its token
  store with `?`: a vault on an unmounted drive stops the app from starting
  (or creates stray folders); no fallback to another known vault.
- `AppData::vault_key` = canonical path with `/` -> `_`: `/a_b` and `/a/b`
  collide, and a vault whose folder is gone has no key (canonicalize fails),
  so `remove_vault` cannot wipe its private data.

## Design

- C: a vault's private-data key is `<name>-<hash12>`: its last folder name,
  sanitized and cut to 40 chars, plus 12 hex digits of the SHA-256 of the path
  *as registered*. No canonicalize at key time (works for a missing folder);
  `add_vault` stores the canonical path when the folder exists, so one folder
  is one spelling. Old `vaults/<key>` folders are orphaned (re-link remote /
  account); nothing deletes them.
- B: `Session::start` picks the first known vault whose folder exists, starting
  with the saved current one; a missing current vault is reported in a startup
  notice (shown in the status line) and the next one is used. A registered
  vault's folder is never created. If no known vault exists on disk, the
  first-run default (`<app data>/notes`) is created and registered.
- A: `immermemo [<dir>] [immermemo://import#...]` in either order, no
  argument is fine. `<dir>` is created if missing (an explicit request),
  registered, and made current. `Session::start` takes a `StartVault`
  (first-run default + optional explicit dir).
- D: every media failure is logged to stderr with its reason, and the UI
  shows one error status per batch ("Could not load N image(s): <reason>");
  the "no LFS server" reason says the vault has no remote linked.

## Steps

- [x] 1 C: `vault_key` + `add_vault` canonicalization + tests
- [x] 2 B: `Session::start` fallback + startup notice + tests
- [x] 3 A: `StartVault`, `main.rs` arg parsing (testable fn) + android caller
- [x] 4 app: show the startup notice
- [x] 5 D: failure log + status, reason text
- [x] 6 docs (`architecture.tmt` if it mentions startup, crate `//!`), clippy,
      tests, `just docs-check`

## Known consequences to tell the user

- Existing registered vaults lose their linked remote/token/account (their
  private data lived under the old key): re-link in Settings. Vaults that are
  in-tree git clones keep their remote (it is read from `.git`).
- `immermemo <typo-dir>` creates the folder (as before).

## Status

All six steps done and verified here: `cargo test --workspace` (298 tests, no
failures), clippy (only the pre-existing `crates/vault/src/size.rs` warning),
`just docs-check`. The CLI was exercised for `--help` and for mistakes only;
the GUI start path (fallback notice on screen, `immermemo <dir>`) is covered
by unit tests of `Session::start` and `cli::parse_args`, not run by hand.
Android's entry point was edited but cannot be compiled on this machine.

Left for the user: try it, then commit and delete this file and the workspace.
`cargo fmt --all` also reformats home-actions code from another commit
(`app/src/callbacks/appearance.rs`, `app/src/tests.rs`); those are restored
each time, not part of this change.
