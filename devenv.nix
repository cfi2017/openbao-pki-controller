{
  pkgs,
  lib,
  config,
  inputs,
  ...
}: {
  packages = [pkgs.git pkgs.kubernetes-helm pkgs.kubectl pkgs.kind];

  languages.rust = {
    enable = true;
  };

  tasks."build:release" = {
    exec = ''
      docker build -t openbao-pki-controller -f Dockerfile .
    '';
  };

  enterTest = ''
    cargo test
  '';

  git-hooks.hooks = {
    # github actions
    actionlint.enable = true;
    action-validator.enable = true;

    # might as well lint nix
    nil.enable = true;
    nixfmt-rfc-style.enable = true;

    # rust
    clippy.enable = true;
    clippy.settings.allFeatures = true;
    clippy.settings.denyWarnings = true;
    cargo-check.enable = true;
    rustfmt.enable = true;

    # helm
    # this one just runs ct lint --all --skip-dependencies
    # chart-testing.enable = true;
  };
}
