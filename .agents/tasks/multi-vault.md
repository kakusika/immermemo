# Switch between multiple vaults

Scope confirmed with the app's author:
- Adding a vault: desktop types an absolute path; Android names a
  subfolder under the app's own internal storage (no Storage Access
  Framework / external-folder picking in this pass).
- Switch UI: tapping the "Notes" title in the list's top bar opens a
  vault-list sheet (an `ActionRow`-style list + an "Add vault" entry).
- Each vault gets its own remote URL, its own gitdir, *and* its own
  access token (not shared across vaults) -- since different vaults
  may point at different hosts entirely.

## Steps
- [x] `appdata.rs`: restructure so gitdir/remote/token all live under
  the same per-vault directory (`base/vaults/<key>/{git,remote.txt,token}`
  instead of gitdir being that directory itself); add
  `known_vaults()`/`add_vault()`/`load_current_vault()`/`save_current_vault()`
  backed by plain-text files at the `base` level (not per-vault), with
  tests
- [x] `dialogs.slint`: generalize `RenameDialog` into a reusable
  text-input dialog (`TextInputDialog`, with a `title` property) so "Add
  vault" doesn't need a near-duplicate component
- [x] New `vault_sheet.slint`: `VaultSheet` -- lists known vaults
  (marking the current one), an "Add vault" row
- [x] `list_pane.slint`: the "Notes" title becomes tappable, firing a
  new `open-vault-sheet()` callback (had to fix an alignment regression
  this introduced -- wrapping the title `Text` in a plain `Rectangle`
  for the `TouchArea` made it center instead of left-align, since a bare
  `Text` centers itself outside a layout; fixed with explicit
  `width`/`horizontal-alignment`)
- [x] `app.slint`: wire the vault sheet's open state, the vault list,
  and the add/switch callbacks through, same shape as the existing
  sheets
- [x] `lib.rs`:
  - `Session` gains `known_vaults: Vec<PathBuf>` and a
    `token_store_for: TokenStoreFactory` (recomputes the per-vault token
    store on switch, since it can't be built once at startup anymore)
  - `run()` takes a *default* vault dir (bootstrap only, used the first
    time there are no known vaults yet -- keeps existing single-vault
    callers/tests working unchanged) and the token-store factory
    instead of one fixed `Arc<dyn TokenStore>`
  - switching vaults reloads notes/remote/token exactly like a fresh
    `run()` would, minus re-reading `known_vaults` itself
  - "Add vault": desktop takes the typed text as a path directly;
    Android joins it as a subfolder name under the same base the
    bootstrap vault already lives under (rejecting empty names, path
    separators, and names that would collide with the app's own
    `vaults`/`vaults.txt`/`current_vault.txt` bookkeeping)
- [x] `main.rs`: build the token-store factory closure instead of one
  `PlainFileTokenStore` (needed a new public `token_path_in` helper,
  since `AppData` itself stays private to the library)
- [x] `android_main`: same, for `AndroidKeystoreTokenStore`
- [x] `examples/snap.rs`: a rendered state for the vault sheet
- [x] `cargo test --workspace`, `cargo clippy --workspace --all-targets`
- [x] Verify on the emulator: added a second vault ("work"), confirmed
  it started empty with its own directory
  (`files/work/`, independent of `files/notes/`), created a note in
  each, and switched back and forth twice confirming each vault only
  ever shows its own note list. Did not verify independent remote/token
  values against two real hosts (would need real credentials for two
  hosts -- the code path is the same one already verified for a single
  vault in mobile-sync.md).

## Deliberately out of scope this pass
- Removing/unlinking a vault from the list
- Android folder picking via SAF (external, user-visible folders) --
  vaults there are subfolders of the app's own storage, matching what
  already exists for the single default vault
