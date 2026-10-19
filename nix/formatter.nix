{
  pkgs,
  tomet,
  ...
}:
{
  projectRootFile = "flake.nix";
  programs = {
    #--- Nix ---
    nixfmt.enable = true;
    statix.enable = true;
    deadnix.enable = true;
    #--- Shell ---
    shfmt.enable = true;
    shellcheck.enable = true;
    #--- Rust ---
    rustfmt.enable = true;
    taplo = {
      enable = true;
      settings = {
        formatting = {
          align_entries = true;
          align_comments = true;
          reorder_keys = false;
        };
        rule = [
          {
            keys = [
              "dependencies"
              "dev-dependencies"
              "build-dependencies"
              "workspace.dependencies"
            ];
            formatting = {
              reorder_keys = true;
            };
          }
        ];
      };
    };
    ktlint.enable = true;
  };

  settings = {
    global.excludes = [
      "*.lock"
      "patches/**"
    ];

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
  };
}
