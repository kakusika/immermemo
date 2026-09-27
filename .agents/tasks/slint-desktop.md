# Slint desktop app (apps/slint)

Trial frontend alongside the Flutter plan. Desktop only for now; no
dependency on mumeum. Loro is used for editor undo/redo only, in memory,
not persisted, never written to git or the working tree.

## Decisions
- Slint features: backend-winit + renderer-skia (same as mumeum, IME verified there).
- `.tmt` text on disk is the only source of truth; Loro is a per-editing-session layer.
- A sync that changes the open note drops the Loro history (new session).

## Steps
- [x] Add `apps/slint` crate to the workspace (not a default member)
- [x] Note list: scan working tree for `.tmt` files
- [x] Editor: plain text editing, save to file
- [x] Loro undo/redo layer (History, unit-tested; Ctrl+Z and Japanese IME undo confirmed by hand)
- [x] Sync button wired (NOT yet run against a real remote): Vault::open / set_remote / sync, show SyncReport
- [x] Conflict display (banner + list badge only; no resolve UI) for notes in `notes_needing_resolution`
- [ ] Verify: cargo build, cargo test --workspace, run the app

## Build
Own devshell (copied from mumeum): `nix develop -c cargo test --workspace`, or direnv (`.envrc`).

## Left
- [x] Sync verified against a local bare remote (apps/slint/src/sync.rs tests)
- [ ] Sync against a real remote over ssh (ssh-agent creds only for now; not tried)
