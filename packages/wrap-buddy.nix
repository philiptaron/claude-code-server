# wrap-buddy patches prebuilt ELF binaries with a stub loader that finds the Nix glibc,
# without rewriting the ELF's interpreter and layout the way patchelf does. Claude Code is
# a `bun build --compile` binary, a runtime with the program appended, and leaving its
# layout alone is the safer way to make it run on NixOS.
#
# This follows numtide/llm-agents.nix's packages/wrapBuddy/package.nix at the same pin.
{
  lib,
  stdenv,
  fetchFromGitHub,
  buildPackages,
  makeSetupHook,
  binutils,
  xxd,
  strace,
}:

let
  src = fetchFromGitHub {
    owner = "Mic92";
    repo = "wrap-buddy";
    rev = "ba5ab56ddc572482c26b2cf08414befc5f66ad40";
    hash = "sha256-NlBlJbQsFKDgUtawzAEGF0iIO/xON2LXGw/axFOU4g8=";
  };

  wrap-buddy = stdenv.mkDerivation {
    pname = "wrap-buddy";
    version = "1.0.0-unstable-ba5ab56";
    inherit src;

    # The patcher runs on the build platform; the stubs it embeds are for the host.
    depsBuildBuild = [ buildPackages.stdenv.cc ];
    nativeBuildInputs = [
      binutils
      xxd
    ];

    makeFlags = [
      "CXX_FOR_BUILD=${buildPackages.stdenv.cc}/bin/c++"
      "BINDIR=$(out)/bin"
      "LIBDIR=$(out)/lib/wrap-buddy"
      "INTERP=${stdenv.cc.bintools.dynamicLinker}"
      "LIBC_LIB=${stdenv.cc.libc}/lib"
    ]
    ++ lib.optional stdenv.hostPlatform.isx86_64 "BUILD_32BIT=1";

    nativeInstallCheckInputs = [ strace ];
    doInstallCheck = true;
    installCheckTarget = "check";
    enableParallelBuilding = true;

    meta = {
      description = "Patch ELF binaries with stub loader for NixOS compatibility";
      homepage = "https://github.com/Mic92/wrap-buddy";
      mainProgram = "wrap-buddy";
      license = lib.licenses.mit;
      platforms = lib.platforms.linux;
    };
  };
in
makeSetupHook {
  name = "wrap-buddy-hook";
  propagatedBuildInputs = [ wrap-buddy ];
  meta = {
    description = "Setup hook that patches ELF binaries with stub loader";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
  };
} "${src}/nix/wrap-buddy-hook.sh"
