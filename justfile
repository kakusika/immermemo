# Requires the dev shell (`nix develop`, or direnv via .envrc) -- these
# recipes assume `emulator`, `avdmanager`, `adb`, `cargo ndk` and `gradle`
# are already on PATH, not just installed somewhere.

avd := "immermemo_test"
device := "pixel_6"
system_image := "system-images;android-34;google_apis;x86_64"
package := "dev.immermemo.app"
activity := package + "/.MainActivity"
dist := "dist"
jniLibs := "android/app/src/main/jniLibs"
apk_raw := "app/android/app/build/outputs/apk/release/app-release.apk"
apk := dist + "/immermemo.apk"
apk_universal := dist + "/immermemo-universal.apk"
apk_arm := dist + "/immermemo-arm64.apk"
apk_x86 := dist + "/immermemo-x86_64.apk"
apk_arm_femtovg := dist + "/immermemo-arm64-femtovg.apk"
apk_x86_femtovg := dist + "/immermemo-x86_64-femtovg.apk"

# List available recipes
default:
    @just --list

# Clean all aggregated build outputs in dist/
clean-dist:
    rm -rf {{dist}}
    @echo "Cleaned {{dist}}/"

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

android_cflags := "-fno-unwind-tables -fno-asynchronous-unwind-tables -ffunction-sections -fdata-sections -DOPENSSL_NO_SM2_PRECOMP -DSQLITE_OMIT_JSON -DSQLITE_OMIT_SHARED_CACHE -DSQLITE_DEFAULT_MEMSTATUS=0 -DSQLITE_DQS=0 -DSQLITE_LIKE_DOESNT_MATCH_BLOBS -DSQLITE_OMIT_UTF16 -DSQLITE_OMIT_AUTOINCREMENT -DSQLITE_OMIT_EXPLAIN -DSQLITE_OMIT_DEPRECATED -DSQLITE_OMIT_LOAD_EXTENSION -DSQLITE_OMIT_PROGRESS_CALLBACK -DSQLITE_OMIT_TRACE -DSQLITE_OMIT_GET_TABLE"
android_rustflags := "-C force-unwind-tables=no -C link-arg=-Wl,--icf=all -C link-arg=-Wl,--gc-sections -C link-arg=-Wl,--strip-all"

# Build one fat APK with both aarch64 (a real phone) and x86_64 (the
# emulator) native libraries, see app/Cargo.toml. Native libs are
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
    cd app && rm -rf {{jniLibs}}
    cd app && RUSTFLAGS="{{android_rustflags}}" CFLAGS="{{android_cflags}}" CXXFLAGS="{{android_cflags}}" cargo ndk -t arm64-v8a -t x86_64 -P 34 -o {{jniLibs}} build --release --lib -p immermemo
    cd app/android && gradle assembleRelease
    @mkdir -p {{dist}}
    cp {{apk_raw}} {{apk_universal}}
    cp {{apk_raw}} {{apk}}
    @echo "Built: {{apk_universal}} (and {{apk}})"

# Build a slim release APK for real devices (aarch64, ~19 MB with FemtoVG GPU renderer)
apk-arm:
    cd app && rm -rf {{jniLibs}}
    cd app && RUSTFLAGS="{{android_rustflags}}" CFLAGS="{{android_cflags}}" CXXFLAGS="{{android_cflags}}" cargo ndk -t arm64-v8a -P 34 -o {{jniLibs}} build --release --lib -p immermemo
    cd app/android && gradle assembleRelease
    @mkdir -p {{dist}}
    cp {{apk_raw}} {{apk_arm}}
    cp {{apk_raw}} {{apk}}
    @echo "Built: {{apk_arm}} (and {{apk}})"

# Build a slim release APK for the emulator (x86_64, FemtoVG GPU renderer)
apk-x86:
    cd app && rm -rf {{jniLibs}}
    cd app && RUSTFLAGS="{{android_rustflags}}" CFLAGS="{{android_cflags}}" CXXFLAGS="{{android_cflags}}" cargo ndk -t x86_64 -P 34 -o {{jniLibs}} build --release --lib -p immermemo
    cd app/android && gradle assembleRelease
    @mkdir -p {{dist}}
    cp {{apk_raw}} {{apk_x86}}
    cp {{apk_raw}} {{apk}}
    @echo "Built: {{apk_x86}} (and {{apk}})"

# Backward-compatible aliases
apk-arm-femtovg: apk-arm
apk-x86-femtovg: apk-x86

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

desktop_cflags := "-fno-unwind-tables -fno-asynchronous-unwind-tables -ffunction-sections -fdata-sections -DOPENSSL_NO_SM2_PRECOMP -DSQLITE_OMIT_JSON -DSQLITE_OMIT_SHARED_CACHE -DSQLITE_DEFAULT_MEMSTATUS=0 -DSQLITE_DQS=0 -DSQLITE_LIKE_DOESNT_MATCH_BLOBS"
desktop_rustflags := "-C force-unwind-tables=no -C link-arg=-Wl,--gc-sections -C link-arg=-fuse-ld=mold"

# Build the release desktop binary and copy it into dist/
desktop:
    RUSTFLAGS="{{desktop_rustflags}}" CFLAGS="{{desktop_cflags}}" CXXFLAGS="{{desktop_cflags}}" cargo build --release -p immermemo --bin immermemo
    @mkdir -p {{dist}}
    cp target/release/immermemo {{dist}}/immermemo-desktop
    @echo "Built: {{dist}}/immermemo-desktop"

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

# Regenerate the translation template from every @tr(...) in app/ui,
# to see what's missing from (or stale in) a .po file under
# app/translations/. Requires `slint-tr-extractor`.
tr-extract:
    python3 scripts/i18n.py extract

# Check for missing or untranslated strings across all languages.
tr-check:
    python3 scripts/i18n.py check

# Extract latest template and sync/append missing strings into .po files.
tr-sync:
    python3 scripts/i18n.py sync

# Generate and update all derived documentation across the workspace.
docs:
    tomet export .
    tomet format -i .

# Check that all documents parse, format cleanly, and match export targets.
docs-check:
    tomet check .
    tomet format --check .
    tomet export --check .
    @if [ -f .writ.tmt ] && command -v twrit >/dev/null 2>&1; then \
        twrit check .; \
    fi
