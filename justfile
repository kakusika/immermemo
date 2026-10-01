# Requires the dev shell (`nix develop`, or direnv via .envrc) -- these
# recipes assume `emulator`, `avdmanager`, `adb` and `cargo apk` are already
# on PATH, not just installed somewhere.

avd := "immermemo_test"
device := "pixel_6"
system_image := "system-images;android-34;google_apis;x86_64"
package := "dev.immermemo.app"
activity := package + "/android.app.NativeActivity"
apk := "target/release/apk/immermemo-slint.apk"

# List available recipes
default:
    @just --list

# One-time setup: create the emulator's AVD. Safe to re-run (--force).
avd-create:
    echo no | avdmanager create avd \
        --name {{avd}} \
        --package "{{system_image}}" \
        --device "{{device}}" \
        --force

# Start the emulator (if one isn't already running) and wait for it to
# fully boot. Leaves it running in the background.
emulator:
    #!/usr/bin/env bash
    set -euo pipefail
    if adb devices | grep -q "device$"; then
        echo "A device is already up (adb devices):"
        adb devices
        exit 0
    fi
    nohup emulator -avd {{avd}} -no-snapshot -no-boot-anim \
        > /tmp/immermemo-emulator.log 2>&1 &
    disown
    adb wait-for-device
    until [ "$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = "1" ]; do
        sleep 2
    done
    echo "Booted. Log: /tmp/immermemo-emulator.log"

# Stop whatever emulator adb currently sees
emulator-stop:
    adb emu kill

# Build the release APK -- one fat APK with both aarch64 (a real phone)
# and x86_64 (the emulator) native libraries, see apps/slint/Cargo.toml.
# --lib is load-bearing: without it, cargo-apk also tries to process the
# crate's desktop `[[bin]]` target as if it were another cdylib, and
# panics ("Bin is not compatible with Cdylib") -- after the APK itself is
# already built and signed, but the nonzero exit still fails this recipe.
apk:
    cd apps/slint && cargo apk build --release --lib

# Build a slim release APK for real devices only (aarch64, ~11 MB)
apk-arm:
    cd apps/slint && cargo apk build --release --lib --target aarch64-linux-android

# Build a slim release APK for the emulator only (x86_64, ~12 MB)
apk-x86:
    cd apps/slint && cargo apk build --release --lib --target x86_64-linux-android

# Install the built APK on whatever device/emulator adb currently sees
install: apk
    adb install -r {{apk}}

# Build, install, and launch
run: install
    adb shell am start -n {{activity}}

# Force-stop the app (e.g. before re-launching after changing saved state
# by hand)
stop:
    adb shell am force-stop {{package}}

# Tail this session's log output from the running app
logs:
    adb logcat --pid="$(adb shell pidof {{package}})"

# A screenshot of whatever's on screen, saved next to this file
screenshot name="screenshot.png":
    adb exec-out screencap -p > {{name}}

# Regenerate the translation template from every @tr(...) in apps/slint/ui,
# to see what's missing from (or stale in) a .po file under
# apps/slint/translations/. Requires `cargo install slint-tr-extractor`.
tr-extract:
    cd apps/slint && find ui -name '*.slint' | xargs slint-tr-extractor -o /tmp/immermemo-slint.pot
    @echo "Wrote /tmp/immermemo-slint.pot"
