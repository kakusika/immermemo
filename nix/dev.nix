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
  ];
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
