# Android APK trial

Goal: build a debug-signed arm64 APK of apps/slint the author can sideload
onto their own phone. No emulator, no Android Studio, no Gradle project.

## Steps
- [x] devshell: Android SDK 34 + NDK 29 via androidenv, license accepted, arm64 only
- [x] devshell: Rust target aarch64-linux-android, cargo-apk, release signed with the local debug key (env in shellHook)
- [x] apps/slint: lib + bin, android_main entry, android backend feature (femtovg)
- [x] git2 (vendored libgit2 + OpenSSL) cross-compiles for Android
- [x] Renderer: femtovg on Android (no Skia)
- [x] Storage: notes under internal_data_path()/notes (sync gitdir NOT wired on Android yet: needs a data dir, no HOME/XDG there)
- [x] Release APK built: target/release/apk/immermemo-slint.apk (~14 MB)
- [ ] Install on the phone and record what it shows (launch, list/editor switch, IME, undo)

## Mobile UI pass (after first phone run: bars hidden under the status bar, buttons too small)
- [x] Screens: list (top bar, 60px rows, FAB) -> editor (back arrow, title, undo/redo bar above the keyboard); wide windows keep two panes
- [x] Safe-area insets and keyboard height from Window properties, updated via `changed` handlers (a binding on the window size is a binding loop)
- [x] Touch targets >= 48px, icons drawn as paths (no icon font needed)
- [x] Preview without a device: `cargo run -p immermemo-slint --example snap -- <prefix> [w] [h] [scale]` (headless software render)
- [ ] Check on the phone: real insets, keyboard avoidance, dark mode, Android back gesture (not handled: it will close the app from the editor)
- [ ] Save errors show only in the list's top bar; not visible from the editor on a narrow window
