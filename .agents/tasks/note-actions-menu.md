# Note actions: long-press menu (Rename, Delete)

Scope (confirmed with the app's author): a long-press on a list row shows
a small action sheet with "Rename" and "Delete"; a real touchscreen/IME
check is still outside what this environment can drive precisely (see
`.agents/tasks/mobile-sync.md`'s note on `adb shell input text`), so
verification here is desktop tests + the `snap` headless-render tool +
an emulator smoke check, not a full on-device pass.

Architecture: mirrors the existing RemoteSheet pattern -- the long-press
action sheet itself is pure, local UI state in `list_pane.slint` (no
Rust round-trip to just show/hide it), but choosing "Rename" or "Delete"
fires a `*-requested(int)` callback up to Rust, which owns the actual
confirm dialogs (`rename-open`/`delete-confirm-open` booleans on `App`,
same as `remote-open`) so Rust can compute the editable file-stem
(`Path::file_stem`, not something worth re-deriving in Slint) and
remember which note is targeted without smuggling an index through
Slint state.

## Steps
- [x] `notes.rs`: `delete(path)` and `rename(path, new_name)` (reject
  empty names and path separators; refuse to overwrite an existing
  file), with tests
- [x] `components.slint`: an `ActionRow` (tappable text row, for the
  action sheet) and a `Colors.danger` token for the delete action
- [x] `list_pane.slint`: long-press detection per row (`Timer` bound to
  `TouchArea.pressed`, fires once), the bottom action-sheet overlay,
  `rename-requested(int)` / `delete-requested(int)` callbacks
- [x] New `dialogs.slint`: `RenameDialog` (LineEdit + Save/Cancel) and
  `ConfirmDialog` (generic enough for delete's "are you sure")
- [x] `app.slint`: wire the new callbacks and the two dialogs' open/draft
  properties through, same shape as `remote-open`/`remote-url-draft`
- [x] `lib.rs`: `Session` gets `rename_target`/`delete_target: Option<usize>`;
  `on_rename_requested`/`on_delete_requested` open the dialogs;
  `on_confirm_rename`/`on_confirm_delete` do the actual file operation,
  refresh the list, and reopen/clear the editor if the affected note was
  the one currently open (mirrors what `finish_sync` already does for
  "the open note disappeared")
- [x] `examples/snap.rs`: a rendered state or two for the new dialogs,
  matching the existing `-remote.ppm` pattern
- [x] `cargo test --workspace`, `cargo clippy --workspace --all-targets`
- [x] Smoke-check on the emulator (`just run`) that long-press actually
  triggers the sheet and both actions work end-to-end

## A real bug the emulator caught: `clicked` still fires after a long hold
First implementation only had the Timer set `menu-index`; a long-press
followed by lifting the finger *also* fired `TouchArea.clicked` (it isn't
time-limited -- releasing inside the area is enough, no matter how long
the hold was), which called `select(i)` and opened the note instead of --
or in addition to -- showing the sheet. Fixed with a per-row
`long-pressed` flag: the Timer sets it before setting `menu-index`, and
`clicked` checks it first (consuming and clearing it) instead of
unconditionally calling `select`.

## Verified on the emulator, end to end
Long-press, Rename (typed a new name, Save, confirmed the file actually
renamed on disk via `adb shell find`), and Delete (confirmed via the
same) all worked correctly against a real device-like environment, not
just the desktop unit tests / `snap` renders.

One environment-specific wrinkle worth recording: driving a long-press
through `adb shell input swipe <x> <y> <x> <y> <duration>` (same start
and end point) does **not** reliably hold the touch down for the full
duration -- it behaved like an ordinary tap. `adb shell input motionevent
DOWN <x> <y>`, a real `sleep`, then `motionevent UP <x> <y>` is what
actually held the press. Even that was inconsistent at a 700ms
sleep between DOWN and UP; 1.5s was reliable. This reads as another
instance of the adb-tooling-precision theme already noted in
mobile-sync.md (input text's shift handling, keycombination) rather than
an app bug -- the underlying Timer mechanism is the same one Slint's own
android-activity backend uses for its built-in long-press (text
selection), so it isn't something invented for this feature. A real
finger should not have the same issue, but this wasn't confirmed on
actual hardware.
