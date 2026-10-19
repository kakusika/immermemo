{
  flake-parts,
  ...
}@inputs:
flake-parts.lib.mkFlake { inherit inputs; } {
  imports = [
    ./base.nix
  ];

  perSystem =
    {
      pkgs,
      craneLib,
      system,
      ...
    }:
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
    };
}
