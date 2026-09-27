{ flake-parts, ... }@inputs:
flake-parts.lib.mkFlake { inherit inputs; } {
  systems = [
    "x86_64-linux"
    "aarch64-linux"
    "aarch64-darwin"
  ];
  imports = [
    inputs.treefmt-nix.flakeModule
  ];

  perSystem =
    { system, pkgs, ... }:
    let
      craneLib = inputs.crane.mkLib pkgs;
    in
    {
      _module.args.pkgs = import inputs.nixpkgs {
        inherit system;
        config = {
          allowUnfree = true;
          # The Android SDK is distributed under Google's license, which nixpkgs makes you accept.
          android_sdk.accept_license = true;
        };
      };
      packages = rec {
        default = immermemo;
        immermemo = pkgs.callPackage ./pkgs/immermemo.nix { inherit craneLib; };
      };

      devShells.default = pkgs.callPackage ./dev.nix {
        inherit inputs;
        fenix = inputs.fenix.packages.${pkgs.stdenv.hostPlatform.system};

        tomet = inputs.tomet.packages.${pkgs.stdenv.hostPlatform.system}.tomet;
        tmtbook = inputs.tomet-book.packages.${pkgs.stdenv.hostPlatform.system}.tmtbook;
        twrit = inputs.twrit.packages.${pkgs.stdenv.hostPlatform.system}.twrit;
      };

      treefmt = import ./formatter.nix {
        tomet = inputs.tomet.packages.${pkgs.stdenv.hostPlatform.system}.tomet;
      };
    };
}
