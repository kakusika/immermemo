# Sync a vault to a real remote (GitHub / Forgejo) from the phone

Adds a remote URL + access token to the app, so Sync actually talks to a
host. HTTPS + personal access token only (no ssh on Android: no agent
there). GitHub, Gitea and Forgejo all accept the token as the HTTPS
password with an empty username, so nothing here special-cases the host.

## Decisions
- Token storage: Android Keystore-backed AES-GCM, via JNI (no Gradle/AAR
  dependency -- plain java.security/javax.crypto, callable straight from
  android-activity's JavaVM).
- Desktop token storage: a plain file, for testing the whole loop without
  a phone. Not for anything real.
- Remote URL itself isn't secret: kept as plain text beside the token.
- One app-data directory holds both the private gitdir and these files,
  computed once (XDG on desktop, internal_data_path on Android).

## Steps
- [x] appdata.rs: app-data-relative paths (gitdir, remote url file)
- [x] credentials.rs: TokenStore trait, PlainFileTokenStore, TokenCredentials (ssh-agent for ssh://, token for https)
- [x] sync.rs: drop the hardcoded DesktopCredentials, take &dyn CredentialProvider
- [x] android_keystore.rs: AndroidKeystoreTokenStore over JNI (cfg(android))
- [x] Cargo.toml: jni dependency (android only), INTERNET permission
- [x] UI: a Remote sheet (URL + token fields) opened from a link icon; Sync uses the saved remote
- [x] lib.rs: wire it all together; main.rs drops the CLI remote arg
- [x] cargo test --workspace (40 tests), build the release APK
- [x] ~~android_tls.rs~~: bundle a CA file so HTTPS verification has
  something to check against at all on Android -- turned out not to work
  at all (see "Second device bug" below); replaced by
  `core/sync/src/tls.rs` + `apps/slint/src/android_cert.rs`

## Verified
- Desktop build, clippy, and the full workspace test suite (includes the
  Keystore-independent token_blob and TokenCredentials/PlainFileTokenStore
  paths). Re-checked 2026-09-27 after resuming: build/test/clippy all still
  clean (one new collapsible_if in the token-save path fixed; the two
  remaining clippy warnings are pre-existing code this task didn't touch).
- **Cross-compiles for Android** (aarch64-linux-android, release APK
  builds and signs) -- this caught two real borrow-checker mistakes in the
  JNI code, now fixed. A clean cross-compile is the only compile-time
  check available for `android_keystore.rs`; it says nothing about
  whether the JNI calls are semantically correct (right class/method/
  signature strings) -- that only shows up at runtime.

## First device bug: "the SSL certificate is invalid"
The first real-device sync attempt (Forgejo over Tailscale, a genuine
Let's Encrypt cert issued via `tailscale cert`) failed HTTPS verification.
Reproduced and ruled out: the cert itself is fine (`curl`, system `git`,
and this workspace's own vendored-openssl `git2` all verify it correctly
from a desktop Linux environment). The actual cause is that `git2::init()`
only *probes* for a CA bundle at a handful of standard Unix paths
(`/etc/ssl/certs/...`) -- none of which exist on Android -- so HTTPS
verification had nothing to check against for *any* host, not just this
one. Fixed by vendoring curl's Mozilla CA extract
(`apps/slint/assets/cacert.pem`, fetched 2026-09-25 from
<https://curl.se/ca/cacert.pem>) and pointing libgit2 at a copy of it under
the app's private storage via `git2::opts::set_ssl_cert_file`, installed
once in `android_main` before anything else runs. Re-fetch and re-vendor
that file by hand when it goes stale -- there's no on-device source to
read it from instead.

Confirmed the bundle is actually linked into the `.so` (121
`BEGIN CERTIFICATE` blocks found in the built library) and that the
release APK still cross-compiles and signs.

## Second device bug (found on the emulator, 2026-09-27): the fix above cannot work at all on Android
`android_tls::install`'s `set_ssl_cert_file` call panics on first launch,
every time, regardless of the file:
```
OpenSSL error: failed to load certificates: error:05880020:x509 certificate routines::BIO lib
```
Root-caused by testing the *same* call against three different targets in
one run (a real single-cert PEM, a garbage text file, and a path that
doesn't exist) -- all three fail with the exact identical error, which
rules out anything about `cacert.pem`'s content and points at the
mechanism itself.

The actual cause: `openssl-src` (the crate that vendor-builds the OpenSSL
`libgit2-sys` links against) passes `no-stdio` to OpenSSL's `Configure`
specifically for Android targets
(`openssl-src-300.6.1+3.6.3/src/lib.rs:279-284`, citing a build failure
without it). `no-stdio` compiles out OpenSSL's file-backed BIO
(`BIO_s_file`), which is what `SSL_CTX_load_verify_locations` (the C
function behind both `set_ssl_cert_file` and `set_ssl_cert_dir`) uses
internally to open a cert file or directory from disk. So neither
function can ever load a CA bundle from a path on Android, on the
emulator or a real phone -- this isn't a bug in this repo's code, it's a
property of how `openssl-src` builds OpenSSL for this target.

`no-stdio`'s rationale traces back to a 2018 issue (a GCC-era NDK linker
failure; the NDK has been Clang-only since 2018, so this may no longer
apply at all) and there's an open upstream issue, filed independently
about six weeks before this session with a matching repro (removing the
flag, building clean, and a real HTTPS push to github.com succeeding),
with the maintainer receptive to dropping the flag entirely:
<https://github.com/alexcrichton/openssl-src-rs/issues/287>. Worth
checking back on periodically, but the fix below doesn't depend on it
landing -- see `core/sync/src/tls.rs`'s module doc for why it's worth
keeping either way.

This means the whole "vendor a CA bundle file, point libgit2 at its path"
approach from the first device bug needed to be replaced, not patched.

### The fix: verify in Rust via `certificate_check`, not libgit2
`git2::RemoteCallbacks::certificate_check` hands the callback the peer's
leaf certificate (`Cert::as_x509()`, raw DER) whenever libgit2's own
verification fails *and* a callback is registered at all -- confirmed by
reading libgit2's `check_certificate` in `transports/httpclient.c`: it
fires even when the low-level OpenSSL check already failed (no CA store
configured counts as "failed"), which is exactly the Android situation.
So instead of asking libgit2 to verify anything, the callback verifies
the chain itself using the `openssl` crate's in-memory APIs (`no-stdio`
only disables the *file-backed* BIO, not memory buffers).

One real complication: `Cert::as_x509()` only ever exposes the leaf
certificate, populated in libgit2's C code from
`SSL_get_peer_certificate`, never `SSL_get_peer_cert_chain` -- so a
normal CA-issued leaf (chains through at least one intermediate before
reaching a trusted root) can't be verified from that alone. Fixed with
AIA chasing: `core/sync/src/tls.rs`'s `ChainVerifier` fetches the missing
intermediate itself over plain HTTP from the leaf's Authority Information
Access "CA Issuers" URL (the same thing a browser does when a server's
handshake doesn't include its own intermediate), same as real TLS
clients do; the trust decision is still made by verifying the resulting
chain up to the bundled Mozilla root store, so a wrong or hostile AIA
response can only ever fail verification, never become trusted itself.

`immermemo_sync::CertificateVerifier` is the new trait (mirrors
`CredentialProvider`); `Vault::set_certificate_verifier` wires it into
both `fetch` and `push`. `apps/slint/src/android_cert.rs` is the
Android-only glue that builds a `ChainVerifier` from the vendored
`cacert.pem` -- desktop passes `None` and keeps using libgit2's ordinary
OS-provided verification, unchanged.

Verified on desktop (`cargo test -p immermemo-sync`) with two tests: a
**real, live** connection to tailscale.com (Let's Encrypt-issued, chains
through a fetched intermediate -- the test asserts the leaf *doesn't*
verify without chasing, so it can't pass trivially) and a synthetic
self-signed certificate (correctly rejected). See that module's doc
comment for the full reasoning.

## NOT verified (needs a device)
- [x] ~~That the CA bundle fix actually resolves the sync error on the
  phone~~ -- superseded, see "Second device bug" above; a real device
  would have hit the exact same failure. The *replacement* fix is what's
  below.
- [x] **The certificate_check-based fix works on-device, against a real
  host, not just the desktop unit tests above** -- confirmed on the
  emulator 2026-09-27: launched, set the remote to
  `https://github.com/octocat/Hello-World.git` (no token), triggered
  Sync, and got all the way to "Sync failed: no access token saved for
  this vault's remote" -- i.e. the TLS handshake succeeded (no
  certificate rejection logged; a rejection would have shown up as this
  crate's own `certificate for <host> not trusted: ...` on stderr, since
  that's only ever printed when `ChainVerifier::trust` returns `false`)
  and the connection got far enough to need real GitHub credentials,
  which is exactly the expected failure for an anonymous request against
  someone else's repo.
- [ ] The Keystore JNI calls actually work: key generation, encrypt,
  decrypt, and that a saved token survives an app restart. A signature
  typo here fails at runtime (an exception `check_exception` should
  surface as a status message) or, in the worst case, could still abort
  the process if a JNI call is malformed in a way `jni` can't catch.
  **Still genuinely needs a real device or a real token**: the emulator
  check above never reached the Keystore-backed credentials path at all
  (it failed one step earlier, at "no token saved").
- [ ] A real sync against a real GitHub or Forgejo repo *with* valid
  credentials (the actual authenticated push/pull) -- the emulator check
  above proves the network/TLS path, not the credentials path.
- [ ] The link icon / Remote sheet on a real touchscreen and IME. Typing
  through `adb shell input text` on the emulator turned out not to
  substitute for this at all -- it silently drops the shift modifier for
  *every* character (both letters and `:` came through unshifted, e.g.
  `ABC:def-GHI` arrived as `abc;def-ghi`), which is a known limitation of
  that specific tool, not of the on-device IME or this app. Confirmed by
  writing the correct URL straight into `remote.txt` instead of through
  the field.

## If the Keystore path breaks on the phone
Fall back to a plain file on Android too (drop `android_keystore.rs`'s use
in `lib.rs`'s `android_main`, swap in `credentials::PlainFileTokenStore`
under `internal_data_path`) to unblock testing the sync loop itself, then
come back to the Keystore code with whatever error message the phone gave.
