{
  lib,
  pkgs,
  mkShell,
  fenix,

  tmtbook,
  tomet,
  twrit,
  ...
}:
let
  rust-toolchain = fenix.combine [
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
    pkg-config

    #[ Develop ]
    tomet
    tmtbook
    twrit
    ##[ Rust ]
    rust-toolchain
    cargo-edit
    cargo-outdated
    cargo-nextest

    ##[ Android ]
    androidSdk.androidsdk
    jdk17
    cargo-apk
    wasm-bindgen-cli

    #[ Runtime ]
    wayland
    libxkbcommon
    fontconfig
    freetype
    libGL
    mesa
    vulkan-loader
    stdenv.cc.cc.lib # Skia's prebuilt binary links against libstdc++

    #[ Build-time only: skia-bindings' bindgen step (see nix/pkgs/immermemo.nix's
    #  own comment on Slint's renderer-skia feature) ]
    llvmPackages.libclang
    python3
    perl # vendored-openssl (git2) builds OpenSSL from source
  ];

  ANDROID_HOME = androidHome;
  ANDROID_SDK_ROOT = androidHome;
  ANDROID_NDK_ROOT = "${androidHome}/ndk-bundle";
  JAVA_HOME = pkgs.jdk17.home;

  # Sideloaded release APKs are signed with the local debug key: enough to
  # install on one's own phone, not for distribution.
  shellHook = ''
    export CARGO_APK_RELEASE_KEYSTORE="$HOME/.android/debug.keystore"
    export CARGO_APK_RELEASE_KEYSTORE_PASSWORD=android
  '';

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
}
