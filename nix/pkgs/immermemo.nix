{
  lib,
  craneLib,
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

  commonArgs = {
    inherit src;
    pname = "immermemo";
    version = "0.1.0";

    # Only the Slint app binary -- crates/sync and crates/merge are plain libraries with
    # no runtime deps of its own, no reason to build it as a separate
    # output.
    cargoExtraArgs = "-p immermemo";

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
