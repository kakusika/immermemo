# Add-vault sheet (name, storage, sync account, repository)

Requested by the user. Today "add vault" is a one-field modal (`TextInputDialog`
in `app/ui/app.slint`): Android takes a name (a folder under the app's
sandbox), desktop a path; no account, no remote. A separate Settings > Accounts
action ("create vault from account") makes a new private repo + a folder under
`<app data>/vaults_repos/`. Linking an existing vault to a remote is only
possible for the *current* vault, in Settings.

## What the user specified

One **sheet** (not a modal) holding, in one screen:
1. **Name**
2. **Storage**: in the app, or local storage (Android shared storage too)
3. **Sync**: yes/no, and the account. If yes: the repository is a new private
   one or an existing one picked from the account's list.

Vault lists show the account (the lists already show the account's avatar and
"provider (@user)" when linked; unlinked ones show a folder icon and "Local").

## Finding that widened the scope

Accounts are not independent today. Every connect flow (GitHub device flow,
Forgejo/GitLab token, the "manual URL" tab) ends by setting up a repository
named `immermemo-notes` and linking the *current vault* to it; the account is
a side effect (`provider::Linked::persist` -> `Session::link_remote`), and the
Settings accounts page derives "connected" from the current vault's remote.
The user said accounts were always meant to be managed independently, so this
task makes them so.

## Design

- **Accounts are independent.** Connecting a provider only registers the
  account and stores its token (`Connected` -> `register_account`); no
  repository, no vault is touched. Settings > Accounts: list, add (GitHub /
  Forgejo / GitLab), delete; each account has "+ Vault".
- **A vault's sync** is a separate, per-vault choice: none, an account +
  repository (new private one, or an existing one from the account's list),
  or a custom remote URL (+ token), which moves out of the accounts page.
  Domain: `Session::set_vault_sync`/`clear_vault_sync` (account id + remote
  url); `unlink_remote` stays.
- **One sheet, two modes** (`VaultSetupSheet`, `AddVaultAdapter`): *add* =
  name, storage, sync; *sync of the current vault* = just the sync part. The
  sync part (toggle, accounts, repository new/existing, custom URL) is one
  component, so it exists once.
- Where a vault lives: *app storage* = `<app data>/folders/<name>`; *local
  storage* = `<chosen folder>/<name>` (an existing folder is fine: that is how
  an existing clone is opened).
- Repos are listed / created on worker threads, with a busy state and errors.
- Android local storage: all-files access (`MANAGE_EXTERNAL_STORAGE`) so the
  vault stays a plain path for libgit2 and std::fs; folder picker is
  `ACTION_OPEN_DOCUMENT_TREE`, its tree URI converted to a path (primary and
  `/storage/<UUID>/` volumes). The pure conversion is unit-tested; the
  permission/picker Kotlin and JNI bridge cannot be run here.

## Steps

- [x] 1 core: vault name rules, `Session::set_vault_sync` / `add_vault` with
      optional sync, tests
- [x] 2 accounts independent (Rust): `Connected` + `register_account`;
      GitHub/token flows become account-only; drop repo setup from them
- [x] 3 Settings > Accounts page: list / add / delete / "+ Vault"; remove the
      vault-centric blocks and the manual-URL tab; remove dead adapter props
- [x] 4 `AddVaultAdapter` + `VaultSetupSheet` (+ sync component) in Slint,
      hosted in a `SheetHost`; snap fixtures
- [x] 5 Rust wiring: open/reset, accounts, repo list/create threads, submit,
      busy/errors, both modes; replace the modal + the old
      create-vault-from-account; entry points (vault sheet, settings)
- [x] 6 vault lists: a clear "does not sync" state
- [x] 7 i18n `.po` catalogs (`just tr-check`)
- [x] 8 Android: tree-URI -> path (pure, tested), permission + picker
      (Kotlin, JNI), manifest, "Choose folder" in the sheet
- [x] 9 docs (`architecture.tmt`), clippy, tests, `just docs-check`

## Known limits to tell the user

- Android picker/permission cannot be run or compiled here.
- An existing local-storage folder plus an existing non-empty repo merges at
  the first sync (two unrelated histories).

## Status

All nine steps done. Verified here: `cargo test --workspace` (319 tests, no
failures), clippy (only the pre-existing `crates/vault/src/size.rs` warning),
`just docs-check`, `just tr-check`-equivalent (`scripts/i18n.py check`: only
the pre-existing appearance entries are untranslated), `cargo ndk -t x86_64
check --lib` (the Android-gated Rust, incl. the JNI bridge and the earlier
`StartVault` change in `android_main`), and `snap` renders of the sheet in
every state.

NOT verified: the Kotlin (`MainActivity.kt`) -- gradle cannot run offline
(no cached aapt2); the Android permission screen and folder picker on a real
device; the sheet on a real device (only the software renderer's snapshots);
creating a repository / listing repositories against a real host.

Left for the user: try it, then commit and delete this file and the
workspace. `cargo fmt --all` reformats home-actions code in
`app/src/callbacks/appearance.rs` and `app/src/tests.rs`; those hunks are
reverted each time, not part of this change.
