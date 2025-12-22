{
  description = "OpenBao PKI Controller";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
    git-hooks.url = "github:cachix/git-hooks.nix";
  };

  outputs =
    { self
    , nixpkgs
    , rust-overlay
    , flake-utils
    , git-hooks
    ,
    }:
    flake-utils.lib.eachDefaultSystem
      (
        system:
        let
          overlays = [ (import rust-overlay) ];
          pkgs = import nixpkgs {
            inherit system overlays;
          };

          rustToolchain = pkgs.rust-bin.stable.latest.default.override {
            extensions = [ "rust-src" "clippy" ];
          };
          pre-commit-check = git-hooks.lib.${system}.run {
            src = ./.;
            hooks = {
              nixpkgs-fmt.enable = true;
              rustfmt = {
                enable = true;
                packageOverrides.cargo = rustToolchain;
                packageOverrides.rustfmt = rustToolchain;
              };
              clippy = {
                enable = false;
                packageOverrides.cargo = rustToolchain;
                packageOverrides.clippy = rustToolchain;
                settings.allFeatures = true;
              };
              actionlint.enable = true;
            };
          };
          controller-bin = pkgs.rustPlatform.buildRustPackage {
            pname = "openbao-pki-controller";
            version = "0.1.0";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;
          };
        in
        {
          packages = rec {
            default = controller-bin;

            inherit controller-bin;

            container =
              let
                version = self.shortRef or "dirty";
              in
              pkgs.dockerTools.buildLayeredImage {
                name = "ghcr.io/cfi2017/openbao-pki-controller";
                tag = version;
                config.Cmd = [ "${controller-bin}/bin/openbao-pki-controller" ];
              };
          };

          checks = {
            inherit pre-commit-check;

            rust-tests = pkgs.rustPlatform.buildRustPackage {
              pname = "openbao-pki-controller-tests";
              version = "0.1.0";
              src = ./.;
              cargoLock.lockFile = ./Cargo.lock;
              buildType = "debug";
              buildPhase = ''
                cargo test --no-fail-fast
              '';
              installPhase = ''touch $out'';
            };

            container-build = pkgs.dockerTools.buildLayeredImage {
              name = "openbao-pki-controller";
              tag = "latest";
              config.Cmd = [ "${controller-bin}/bin/openbao-pki-controller" ];
            };
          };

          devShells.default = pkgs.mkShell {
            inherit (pre-commit-check) shellHook;

            buildInputs = with pkgs; [
              rustToolchain
              rust-analyzer

              git
              kubernetes-helm
              kubectl
              kind

              cargo-machete
              cargo-outdated
              cargo-watch
              cargo-edit

              skopeo
            ];

            RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
          };

          apps = {
            test = {
              type = "app";
              program = toString (pkgs.writeShellScript "test" ''
                ${rustToolchain}/bin/cargo test --all-features
              '');
            };

            fmt = {
              type = "app";
              program = toString (pkgs.writeShellScript "fmt" ''
                ${rustToolchain}/bin/cargo fmt
              '');
            };

            clippy = {
              type = "app";
              program = toString (pkgs.writeShellScript "clippy" ''
                ${rustToolchain}/bin/cargo clippy --all-features -- -D warnings
              '');
            };

            registry-login = {
              type = "app";
              program = toString (pkgs.writeShellScript "registry-login" ''
                exec ${pkgs.skopeo}/bin/skopeo login \
                  ghcr.io \
                  --username "$REGISTRY_USER" \
                  --password-stdin
              '');
            };

            push-container = {
              type = "app";
              program = toString (pkgs.writeShellScript "push-container" ''
                set -euo pipefail
                IMAGE_TAR=$(nix build --no-link --print-out-paths .#container)
                skopeo copy docker-archive:$IMAGE_TAR docker://ghcr.io/cfi2017/openbao-pki-controller:${self.shortRev or "latest"}
              '');
            };
          };
        }
      );
}
