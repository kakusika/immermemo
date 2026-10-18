{
  flake-parts,
  ...
}@inputs:
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
      sisho = inputs.sisho.packages.${system};
      sishoLib = inputs.sisho.legacyPackages.${system};
    in
    {
      _module.args.pkgs = import inputs.nixpkgs {
        inherit system;
        config = {
          allowUnfree = true;
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

        # sisho, tomet and twrit, with the language server and the book generator.
        tools = sishoLib.combine [
          sisho.toolchain
          sisho.tomet-lsp
          sisho.tomet-shelf
        ];
      };

      treefmt = import ./formatter.nix {
        inherit pkgs;
        tomet = sisho.tomet;
      };
    };
}
