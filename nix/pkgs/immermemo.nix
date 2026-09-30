{
  lib,
  craneLib,
  fetchurl,
  stdenv,

  #= Dependencies
  pkg-config,
  fontconfig,
  freetype,
  wayland,
  libxkbcommon,
  libGL,
  mesa,
  vulkan-loader,
  llvmPackages,
  python3,
  perl,
}:
let
  src = ../..;

  # skia-bindings downloads a prebuilt Skia at build time, which the Nix sandbox
  # forbids (and has no curl for). Fetch it here as a fixed-output derivation and
  # point skia-bindings at it with a file:// URL. The name encodes the skia-bindings
  # version and the feature set Slint enables, so bump it together with the `slint`
  # dependency: the new URL is printed in the `skia-bindings` build output
  # (`FROM:`) of a `cargo build` that has network access.
  skiaBinaries = fetchurl {
    url = "https://github.com/rust-skia/skia-binaries/releases/download/0.153.3/skia-binaries-b7f043e0b1e2a850e702-x86_64-unknown-linux-gnu-ganesh-gl-jpegd-jpege-pdf-vulkan.tar.gz";
    hash = "sha256-nr5MRIzJ94m60kHL0Rc5zfdZ4n21cZwjp2NeCW1YB7M=";
  };
  commonArgs = {
    inherit src;
    pname = "immermemo";
    version = "0.1.0";

    # Only the Slint app binary -- crates/sync and crates/merge are plain libraries with
    # no runtime deps of its own, no reason to build it as a separate
    # output.
    cargoExtraArgs = "-p immermemo-slint";

    SKIA_BINARIES_URL = "file://${skiaBinaries}";

    # libclang/python3 are build-time only -- `skia-bindings` (pulled in
    # by Slint's `renderer-skia` feature, see the workspace `Cargo.toml`'s
    # own comment on why Skia over the stock FemtoVG renderer) runs
    # bindgen against a prebuilt Skia binary at build time, which needs
    # both.
    nativeBuildInputs = [
      pkg-config
      llvmPackages.libclang
      python3
      perl # vendored-openssl (git2) builds OpenSSL from source
    ];

    # Same minimal set nix/dev.nix uses, proven against `cargo check`
    # during scaffolding -- Slint's winit backend needs these at link
    # time, nothing Qt-specific is required. `stdenv.cc.cc.lib` is new:
    # Skia's prebuilt binary links against libstdc++, which nothing else
    # in this list otherwise pulls in.
    buildInputs = [
      fontconfig
      freetype
      wayland
      libxkbcommon
      libGL
      mesa
      vulkan-loader
      stdenv.cc.cc.lib
    ];
  };
  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
in
craneLib.buildPackage (
  commonArgs
  // {
    inherit cargoArtifacts;
    doCheck = false;

    meta = {
      description = "Mobile note app that writes Tomet notes and syncs them over git";
      mainProgram = "immermemo";
      license = with lib.licenses; [
        mit
        asl20
      ];
    };
  }
)
