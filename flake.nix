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
      eachLinuxSystem = f: lib.genAttrs linuxSystems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = eachLinuxSystem (pkgs: {
        ccd-cli-fs = pkgs.callPackage ./packages/ccd-cli-fs.nix { };
        default = self.packages.${pkgs.stdenv.hostPlatform.system}.ccd-cli-fs;
      });

      devShells = eachLinuxSystem (pkgs: {
        default = pkgs.mkShell {
          inputsFrom = [ self.packages.${pkgs.stdenv.hostPlatform.system}.ccd-cli-fs ];
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
