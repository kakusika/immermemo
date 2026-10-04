# Requires the dev shell (`nix develop`, or direnv via .envrc) -- these
# recipes assume `emulator`, `avdmanager`, `adb`, `cargo ndk` and `gradle`
# are already on PATH, not just installed somewhere.

avd := "immermemo_test"
device := "pixel_6"
system_image := "system-images;android-34;google_apis;x86_64"
package := "dev.immermemo.app"
activity := package + "/android.app.NativeActivity"
jniLibs := "android/app/src/main/jniLibs"
apk := "immermemo/android/app/build/outputs/apk/release/app-release.apk"

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

# Build one fat APK with both aarch64 (a real phone) and x86_64 (the
# emulator) native libraries, see immermemo/Cargo.toml. Native libs are
# stored uncompressed and page-aligned (Gradle's
# `packaging.jniLibs.useLegacyPackaging = false`, see android/app/build.gradle.kts),
# so there's no second install-time-extracted copy on top of the APK's own
# size -- but bundling both ABIs' full-size libs in one file still makes
# this the biggest of the three on-device footprints (~51 MB). Only useful
# for sideloading onto a device whose ABI you don't know ahead of time;
# `install`/`run` use `apk-auto` instead, which picks a single ABI.
# --lib is load-bearing: without it, cargo-ndk also tries to cross-compile
# the crate's desktop `[[bin]]` target, which doesn't link for Android.
apk:
    cd immermemo && rm -rf {{jniLibs}}
    cd immermemo && cargo ndk -t arm64-v8a -t x86_64 -P 34 -o {{jniLibs}} build --release --lib -p immermemo
    cd immermemo/android && gradle assembleRelease

# Build a slim release APK for real devices only (aarch64, ~24 MB on-device,
# no duplicate extracted copy -- see the `apk` recipe's comment)
apk-arm:
    cd immermemo && rm -rf {{jniLibs}}
    cd immermemo && cargo ndk -t arm64-v8a -P 34 -o {{jniLibs}} build --release --lib -p immermemo
    cd immermemo/android && gradle assembleRelease

# Build a slim release APK for the emulator only (x86_64, ~26 MB on-device)
apk-x86:
    cd immermemo && rm -rf {{jniLibs}}
    cd immermemo && cargo ndk -t x86_64 -P 34 -o {{jniLibs}} build --release --lib -p immermemo
    cd immermemo/android && gradle assembleRelease

# Build for whatever single device/emulator adb currently sees, picking
# its ABI automatically instead of bundling both (what `install`/`run`
# use by default -- see the `apk` recipe for when the universal build is
# the right call instead).
apk-auto:
    #!/usr/bin/env bash
    set -euo pipefail
    abi="$(adb shell getprop ro.product.cpu.abi | tr -d '\r')"
    case "$abi" in
        arm64-v8a) just apk-arm ;;
        x86_64)    just apk-x86 ;;
        *)
            echo "error: unsupported device ABI '$abi' (expected arm64-v8a or x86_64)" >&2
            exit 1
            ;;
    esac

# Install the built APK on whatever device/emulator adb currently sees
install: apk-auto
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

# Regenerate the translation template from every @tr(...) in immermemo/ui,
# to see what's missing from (or stale in) a .po file under
# immermemo/translations/. Requires `cargo install slint-tr-extractor`.
tr-extract:
    cd immermemo && find ui -name '*.slint' | xargs slint-tr-extractor -o /tmp/immermemo-slint.pot
    @echo "Wrote /tmp/immermemo-slint.pot"
