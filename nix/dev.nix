{
  lib,
  pkgs,
  mkShell,
  fenix,
  tmtbook,
  tomet,
  tomet-lsp,
  twrit,
  ...
}:
let
  rustToolchain = fenix.combine [
    (fenix.stable.withComponents [
      "cargo"
      "clippy"
      "rustc"
      "rust-src"
    ])
    fenix.targets.aarch64-linux-android.stable.rust-std
    # For the emulator only: a real phone is always arm64-v8a, but an x86_64
    # system image runs at native speed under KVM instead of translating
    # ARM -- close enough for anything that isn't CPU-architecture-specific
    # (JNI class/method-signature bugs included), not a stand-in for a real
    # device.
    fenix.targets.x86_64-linux-android.stable.rust-std
  ];
  androidSdk = pkgs.androidenv.composeAndroidPackages {
    # Must match the `-P`/`--platform` given to `cargo ndk` in the `apk`
    # justfile recipe: that value also picks which `android.jar` Slint's
    # own build-time Java helper compiles against (not just the native API
    # level), and anything below compileSdk/targetSdk (34) is missing
    # symbols (`WindowInsets.Type`, `Insets`, ...) the helper references
    # even inside `Build.VERSION.SDK_INT` guards -- javac still needs them
    # in the compile-time classpath.
    platformVersions = [ "34" ];
    buildToolsVersions = [ "34.0.0" ];
    includeNDK = true;
    # abiVersions only selects which system images get fetched (real-device
    # cross-compiling goes through the NDK and the Rust targets above,
    # neither of which reads this list) -- arm64-v8a would just be a second,
    # unused ~1.6 GB system-image download alongside the one the emulator
    # actually runs.
    abiVersions = [ "x86_64" ];
    includeEmulator = true;
    includeSystemImages = true;
    systemImageTypes = [ "google_apis" ];
  };
  androidHome = "${androidSdk.androidsdk}/libexec/android-sdk";
in
mkShell {
  buildInputs = with pkgs; [
    #= Develop
    tomet
    tomet-lsp
    tmtbook
    twrit
    slint-lsp
    slint-viewer
    just
    #== Build
    pkg-config
    llvmPackages.libclang
    python3
    perl # vendored-openssl (git2) builds OpenSSL from source
    #== Rust
    rustToolchain
    cargo-edit
    cargo-outdated
    cargo-nextest
    #== Android
    androidSdk.androidsdk
    jdk17
    wasm-bindgen-cli
    gradle
    cargo-ndk

    #= Runtime
    #== Wayland
    wayland
    libxkbcommon
    fontconfig
    freetype
    stdenv.cc.cc.lib # Skia's prebuilt binary links against libstdc++
    #== Graphics
    libGL
    mesa
    vulkan-loader
    vulkan-validation-layers
    vulkan-tools
  ];

  ANDROID_HOME = androidHome;
  # ANDROID_SDK_ROOT = androidHome;
  ANDROID_NDK_ROOT = "${androidHome}/ndk-bundle";
  JAVA_HOME = pkgs.jdk17.home;

  PKG_CONFIG_PATH = lib.makeSearchPathOutput "dev" "lib/pkgconfig" [
    pkgs.fontconfig
    pkgs.freetype
    pkgs.wayland
    pkgs.libxkbcommon
  ];

  LD_LIBRARY_PATH =
    lib.makeLibraryPath [
      pkgs.wayland
      pkgs.libxkbcommon
      pkgs.fontconfig
      pkgs.freetype
      pkgs.mesa
      pkgs.libGL
      pkgs.vulkan-loader
      pkgs.stdenv.cc.cc.lib
    ]
    + ":/run/opengl-driver/lib";

  # Sideloaded release APKs are signed with the local debug key: enough to
  # install on one's own phone, not for distribution.
  shellHook = ''
    export CARGO_APK_RELEASE_KEYSTORE="$HOME/.android/debug.keystore"
    export CARGO_APK_RELEASE_KEYSTORE_PASSWORD=android

    export ANDROID_AVD_HOME="$HOME/.android/avd"

    echo "🧪 Rust Android Tomet"
  '';
}
