# One Claude Code version from outside versions.json, for ccd-cli-fs-builder:
#
#   nix build --impure --file build.nix --argstr nixpkgs PATH --argstr system SYSTEM \
#     --argstr version VERSION --argstr hash SRI
{
  nixpkgs,
  system,
  version,
  hash,
}:

let
  pkgs = import nixpkgs {
    inherit system;
    # Not the user's ~/.config/nixpkgs/overlays, which an impure import reads by default.
    overlays = [ ];
    config.allowUnfreePredicate = pkg: pkgs.lib.getName pkg == "claude-code";
  };
in
pkgs.callPackage ./package.nix {
  inherit version;
  hashes.${system} = hash;
  wrapBuddy = pkgs.callPackage ../wrap-buddy.nix { };
}
