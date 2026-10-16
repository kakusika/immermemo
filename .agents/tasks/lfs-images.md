# Fetch Git LFS images

Follows "Media: icons, thumbnails, banners" (done, folded into `crates/media`'s
`//!` and `tmtroot/architecture.tmt`). The user's vault routes images through
Git LFS (`*.png filter=lfs ...`); libgit2, which the app syncs with, never
runs the LFS filter, so a vault image on disk is a ~130-byte pointer file and
nothing can be drawn. The user chose to *fetch* the objects.

## Design

- A pointer file's identity is `lfs:<oid>` (content-addressed, independent of
  path/mtime), so one object is one cache entry however many files point at it,
  and a cached thumbnail needs no network.
- Reading a pointer downloads the object through the LFS batch API
  (`<remote>.git/info/lfs/objects/batch`, `operation: download`), then GETs the
  `actions.download.href` with the headers the server gave. The bytes are
  verified against the pointer's sha256 and size before use. Objects bigger than
  a cap are refused before downloading.
- Nothing is written to the vault or git: the object lives only in the disk
  cache as the thumbnail made from it (the original is not kept).
- Auth: the same token and username convention sync uses
  (`https_basic_auth_for_remote`), read in the worker when a download actually
  happens, not on the UI thread. SSH remotes map to the https LFS endpoint, as
  git-lfs does; only token auth works.
- `Fetcher` stays a stateless HTTP client (new default-off `request` method for
  POST/headers); the per-vault part (endpoint + auth) is data, `LfsRemote`,
  passed down with the job.

## Steps

- [x] 1 `media/lfs.rs`: `LfsRemote` (endpoint from a git remote URL, auth),
      batch request/response JSON, `download` with sha256+size check and caps;
      tests with a fake HTTP client (no network)
- [x] 2 `media`: `Fetcher::request`, `identity()` = `lfs:<oid>` for a pointer,
      `read()` downloads instead of failing, pass `LfsRemote` through
      `produce`; tests
- [x] 3 app: `UreqFetcher::request`, `LfsRemote` built from the session
      (remote url + token store), carried by `MediaBase`/`Thumbs::picture`/`Job`
- [x] 4 tests (app-level with a fake fetcher), clippy, `just docs-check`,
      `architecture.tmt` + `crates/media` `//!` updated (LFS section now says
      "fetched")
- [ ] 5 try against the real vault: NOT done by the agent on purpose (it would
      send the user's stored token to the LFS server); the user tries it in the app

## Known limits to tell the user

- Only token (HTTPS basic) auth; SSH-key-only remotes cannot download.
- Offline or a failed download shows the placeholder and is retried after the
  15 min failure memory.
- No upload, no `git lfs pull` into the vault.

## Status

Steps 1-4 done and verified here: `cargo test --workspace` (all green),
clippy (only the pre-existing `crates/vault/src/size.rs` warning),
`just docs-check`. Never exercised against a real LFS server: the unit tests
use a fake client. Left for the user's check on a device; then fold nothing
further (design lives in `crates/media`'s `//!` and `architecture.tmt`),
commit, delete this file and the workspace.

`cargo fmt --all --check` fails in `app/src/callbacks/appearance.rs` and
`app/src/tests.rs` (home-actions code from another commit); left alone.
