# Depend on origami-frameworks' origami-mobile instead of the local crate

`crates/origami-mobile` has been migrated into `origami-frameworks` as
`origami-mobile` (commit `7a354d3`, pushed). Its own `src/lib.rs` doc
comment always said this move would happen "with no restructuring" once
origami-frameworks' own `origami` crate matured -- it has. Switch this
repo to depend on the upstream crate and drop the local copy.

The Slint import prefix (`@origami-mobile/...`) does not change --
`apps/slint/ui/**` needs zero `.slint` edits. Only the Rust-side crate
name/env var changes.

## Steps

- [x] Root `Cargo.toml`: removed `"crates/origami-mobile"` from
      `[workspace] members`; removed `immermemo-origami-mobile`; added
      `origami-mobile = { git = "https://github.com/kakusika/origami-frameworks.git" }`
      under `#= Origami`.
- [x] `apps/slint/Cargo.toml`: `immermemo-origami-mobile` ->
      `origami-mobile = { workspace = true }`, moved into the `#= Origami`
      group.
- [x] `apps/slint/build.rs`: `DEP_IMMERMEMO_ORIGAMI_MOBILE_UI_DIR` ->
      `DEP_ORIGAMI_MOBILE_UI_DIR`; comment updated.
- [x] Deleted `crates/origami-mobile/` (superseded).
- [x] `cargo build -p immermemo-slint` and `cargo test --workspace` both
      pass.
- [ ] Commit (no `Co-Authored-By`, per this repo's own `AGENTS.md`).
      Push only once confirmed with user.
