{ inputs, ... }:
{
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
      sisho = inputs.sisho.packages.${system};
      sishoLib = inputs.sisho.legacyPackages.${system};
      tools = sishoLib.combine [
        sisho.toolchain
        sisho.tomet-lsp
        sisho.tomet-shelf
      ];
    in
    {
      _module.args.craneLib = inputs.crane.mkLib pkgs;
      _module.args.sisho = sisho;
      _module.args.sishoLib = sishoLib;
      _module.args.tools = tools;

      treefmt = import ./formatter.nix {
        inherit pkgs;
        inherit (sisho) tomet;
      };

      devShells.default = pkgs.callPackage ./dev.nix {
        inherit inputs tools;
        fenix = inputs.fenix.packages.${pkgs.stdenv.hostPlatform.system};
      };
    };
}
