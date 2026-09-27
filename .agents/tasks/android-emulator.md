# Run the Android build in an emulator instead of only a real phone

`nix/dev.nix` already has an uncommitted, half-finished change (from a prior
session) turning on the emulator + an x86_64 system image alongside the
existing arm64-v8a device tooling. Finish wiring that up so the
device-only checklist items in `.agents/tasks/android-apk.md` and
`.agents/tasks/mobile-sync.md` can be exercised without a physical phone.

`/dev/kvm` exists and the user is in the `kvm` group, so the emulator
should get native-speed KVM acceleration for the x86_64 image.

## Steps
- [x] `nix develop` picks up the changed `nix/dev.nix` cleanly (SDK +
  emulator + x86_64 system image download/build; first run only) --
  fixed the uncommitted diff first: it had left `arm64-v8a` in
  `abiVersions` alongside `x86_64`, which downloads a second, unused
  ~1.6 GB system image (`abiVersions` only selects system images, not
  the NDK/real-device build path -- confirmed by grepping nixpkgs'
  `compose-android-packages.nix`). Trimmed to `abiVersions = [ "x86_64" ]`.
- [x] Create an AVD (avdmanager, x86_64, google_apis, API 34, Pixel 6
  profile) with KVM accel -- `immermemo_test`, note it lands under
  `~/.config/.android/avd`, not `~/.android/avd`; the emulator binary
  needs `ANDROID_AVD_HOME` pointed there explicitly or it won't find it
- [x] Booted with a window (this environment has a Wayland session), `adb
  devices` sees it, `sys.boot_completed` reached
- [x] Built the release APK for both aarch64 and x86_64 in one fat APK
  (added `x86_64-linux-android` to `build_targets` in
  `apps/slint/Cargo.toml`) -- cross-compiles and signs; `cargo apk`
  panics afterwards on the crate's separate `[[bin]]` desktop target
  ("Bin is not compatible with Cdylib") but the APK itself is already
  built and signed by that point, harmless
- [x] Installed and launched on the emulator -- **crashed on first
  launch**, see "Second device bug" in mobile-sync.md: `set_ssl_cert_file`
  cannot work on Android at all (`openssl-src` builds OpenSSL with
  `no-stdio` for Android, which removes file-based cert loading
  entirely). Root-caused via logcat + a temporary diagnostic build (a
  real cert, a garbage file, and a nonexistent path all fail identically
  -- reverted before committing).
- [x] Fixed (see mobile-sync.md's "Second device bug" for the full
  writeup): replaced the file-based CA bundle with a `certificate_check`
  callback that verifies the chain itself in memory, AIA-chasing the
  missing intermediate. Rebuilt, reinstalled, relaunched -- app now
  starts and renders (screenshot: the Remote sheet, with the on-screen
  keyboard up). Also fixed the one new clippy warning
  (`collapsible_if`) this introduced in `lib.rs`.
- [x] Ran the actual sync path against a real host from the emulator:
  set the remote to `https://github.com/octocat/Hello-World.git`,
  triggered Sync, got to "no access token saved" -- past the TLS
  handshake, failing only where it's supposed to (see mobile-sync.md).
- [ ] `android-apk.md`'s UI checklist (insets, keyboard avoidance, dark
  mode, back gesture) is still open -- the app renders now, so this is
  reachable, just not gone through yet.
- Typing through `adb shell input text` cannot be used for the "Remote
  sheet on real touch+IME" check: it drops the shift modifier for every
  character (letters and `:` both), a tool limitation, not an app or IIME
  one. That check still needs either a real device or driving the
  emulator's keyboard some other way (e.g. `input keycombination`, which
  also didn't reproduce shifted characters correctly in a quick try here
  -- not investigated further).

## Notes
- Emulator is x86_64, a real phone is always arm64-v8a -- fine for
  anything that isn't CPU-architecture-specific (JNI class/method
  signature bugs included per dev.nix's own comment), not a full
  substitute for the real-device checks already marked as needing one.
