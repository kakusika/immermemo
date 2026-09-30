{ pkgs, tomet, ... }:
{
  projectRootFile = "flake.nix";
  programs = {
    #= Nix
    nixfmt.enable = true;
    statix.enable = true;
    deadnix.enable = true;
    #= Shell
    shfmt.enable = true;
    shellcheck.enable = true;

    #= Main
    rustfmt.enable = true; # Rust
    taplo.enable = true; # Toml
  };
  settings = {
    # https://github.com/numtide/treefmt-nix/issues/171
    global.excludes = [ ];

    formatter = {
      tomet = {
        command = "${tomet}/bin/tomet";
        options = [
          "format"
          "-i"
        ];
        includes = [ "*.tmt" ];
      };
      slint = {
        command = "${pkgs.slint-lsp}/bin/slint-lsp";
        options = [
          "format"
          "-i"
        ];
        includes = [ "*.slint" ];
      };
    };

    shfmt = {
      includes = [ "*.sh" ];
    };

    rustfmt = {
      includes = [ "*.rs" ];
    };
  };
}
