{
  description = "Nix-built Claude Code CLIs for the Claude desktop app's SSH remote";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs";

  outputs =
    { self, nixpkgs }:
    let
      inherit (nixpkgs) lib;
      linuxSystems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      systems = linuxSystems ++ [ "aarch64-darwin" ];

      # The Claude Code versions this flake ships, newest first; see
      # packages/claude-code/update.py.
      versions = lib.importJSON ./packages/claude-code/versions.json;
      newest = lib.head (lib.sort (a: b: lib.versionOlder b a) (lib.attrNames versions));
      # claude-code_2_1_322, as a flake attribute cannot hold dots unquoted.
      attrName = version: "claude-code_" + lib.replaceStrings [ "." ] [ "_" ] version;

      pkgsFor =
        system:
        import nixpkgs {
          inherit system;
          config.allowUnfreePredicate = pkg: lib.getName pkg == "claude-code";
          overlays = [ self.overlays.default ];
        };
      eachSystem = systems: f: lib.genAttrs systems (system: f (pkgsFor system));
    in
    {
      # claude-code-versions."2.1.322" and so on, ccd-cli-fs, and ccd-cli-fs-builder. Claude
      # Code is unfree, so the nixpkgs this is applied to must allow it.
      overlays.default = final: prev: {
        claude-code-versions = lib.mapAttrs (
          version: hashes:
          final.callPackage ./packages/claude-code/package.nix {
            inherit version hashes;
            wrapBuddy = final.callPackage ./packages/wrap-buddy.nix { };
          }
        ) versions;
        ccd-cli-fs = final.callPackage ./packages/ccd-cli-fs.nix { };
        # Builds with this flake's nixpkgs, which `follows` can make the consumer's. Not
        # final.path: interpolating that copies all of nixpkgs into the store again.
        ccd-cli-fs-builder = final.callPackage ./packages/ccd-cli-fs-builder.nix {
          nixpkgsPath = nixpkgs.outPath;
        };
      };

      packages = eachSystem systems (
        pkgs:
        lib.mapAttrs' (version: lib.nameValuePair (attrName version)) pkgs.claude-code-versions
        // {
          claude-code = pkgs.claude-code-versions.${newest};
        }
        // lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
          inherit (pkgs) ccd-cli-fs ccd-cli-fs-builder;
          default = pkgs.ccd-cli-fs;
        }
      );

      # The same versions, keyed by version string.
      legacyPackages = eachSystem systems (pkgs: {
        inherit (pkgs) claude-code-versions;
      });

      devShells = eachSystem linuxSystems (pkgs: {
        default = pkgs.mkShell {
          inputsFrom = [ pkgs.ccd-cli-fs ];
          packages = with pkgs; [
            clippy
            rustfmt
            rust-analyzer
            nixfmt
            python3
          ];
        };
      });
    };
}
