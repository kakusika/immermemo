{
  lib,
  pkgs,
  mkShell,
  fenix,
  tools,
  ...
}:
let
  rustToolchain = fenix.combine [
    (fenix.stable.withComponents [
      "cargo"
      "clippy"
      "rustc"
      "rust-src"
      "rustfmt"
      "rust-analyzer"
    ])
    fenix.targets.aarch64-linux-android.stable.rust-std
    fenix.targets.x86_64-linux-android.stable.rust-std
  ];
  androidSdk = pkgs.androidenv.composeAndroidPackages {
    platformVersions = [ "34" ];
    buildToolsVersions = [ "34.0.0" ];
    includeNDK = true;
    abiVersions = [ "x86_64" ];
    includeEmulator = true;
    includeSystemImages = true;
    systemImageTypes = [ "google_apis" ];
  };
  androidHome = "${androidSdk.androidsdk}/libexec/android-sdk";
in
mkShell {
  hardeningDisable = [ "fortify" ];
  buildInputs = with pkgs; [
    #= Develop
    tools
    just
    #== Slint
    slint-lsp
    slint-viewer
    slint-tr-extractor
    just
    #== Build
    pkg-config
    cmake
    llvmPackages.libclang
    python3
    #== Rust
    rustToolchain
    cargo-edit
    cargo-outdated
    cargo-nextest
    wasm-bindgen-cli

    #== Android
    androidSdk.androidsdk
    jdk17
    gradle
    cargo-ndk

    #= Runtime
    #== Crypto / Network
    openssl
    #== Wayland & Desktop
    wayland
    libxkbcommon
    libinput
    pipewire
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
  ANDROID_NDK_ROOT = "${androidHome}/ndk-bundle";
  JAVA_HOME = pkgs.jdk17.home;

  LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
  BINDGEN_EXTRA_CLANG_ARGS =
    "-isystem ${pkgs.stdenv.cc.cc}/include/c++/${pkgs.stdenv.cc.cc.version} "
    + "-isystem ${pkgs.stdenv.cc.cc}/include/c++/${pkgs.stdenv.cc.cc.version}/${pkgs.stdenv.hostPlatform.config} "
    + "-isystem ${pkgs.stdenv.cc.libc.dev}/include";

  PKG_CONFIG_PATH = lib.makeSearchPathOutput "dev" "lib/pkgconfig" [
    pkgs.openssl
    pkgs.fontconfig
    pkgs.freetype
    pkgs.wayland
    pkgs.libxkbcommon
  ];

  LD_LIBRARY_PATH =
    lib.makeLibraryPath [
      pkgs.openssl
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

  shellHook = ''
    export CARGO_APK_RELEASE_KEYSTORE="$HOME/.android/debug.keystore"
    export CARGO_APK_RELEASE_KEYSTORE_PASSWORD=android
    export ANDROID_AVD_HOME="$HOME/.android/avd"
    export ANDROID_BUILD_TOOLS_VERSION="34.0.0"

    echo "📱 Rust + Slint + Android dev shell"
  '';
}
