{ lib, rustPlatform }:

rustPlatform.buildRustPackage {
  pname = "ccd-cli-fs";
  version = (lib.importTOML ../Cargo.toml).package.version;
  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../src
    ];
  };
  # From the lock file rather than a cargoHash, so a Dependabot bump of Cargo.lock builds
  # without anyone updating a hash.
  cargoLock.lockFile = ../Cargo.lock;

  meta = {
    description = "FUSE filesystem that serves Nix-built Claude Code CLIs to the Claude desktop app's SSH remote";
    license = lib.licenses.mit;
    mainProgram = "ccd-cli-fs";
    platforms = lib.platforms.linux;
  };
}
