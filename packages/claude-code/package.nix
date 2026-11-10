# Claude Code from Anthropic's prebuilt release binaries, packaged the way
# numtide/llm-agents.nix packages it (packages/claude-code/package.nix there): the
# uncompressed binary from the release bucket, made runnable with wrap-buddy rather than
# autoPatchelf, and wrapped once.
#
# One version per call; versions.json holds the versions this flake ships and their hashes,
# which update.py takes from each release's manifest.json.
{
  lib,
  stdenv,
  fetchurl,
  makeWrapper,
  wrapBuddy,
  versionCheckHook,
  bubblewrap,
  socat,
  version,
  # nix system -> SRI hash of that platform's binary
  hashes,
  disableTelemetry ? false,
}:

let
  platforms = {
    x86_64-linux = "linux-x64";
    aarch64-linux = "linux-arm64";
    aarch64-darwin = "darwin-arm64";
  };
  system = stdenv.hostPlatform.system;
  platform = platforms.${system} or (throw "claude-code: unsupported system ${system}");
in
stdenv.mkDerivation {
  pname = "claude-code";
  inherit version;

  src = fetchurl {
    url = "https://downloads.claude.ai/claude-code-releases/${version}/${platform}/claude";
    hash = hashes.${system} or (throw "claude-code ${version}: no hash for ${system}");
  };

  dontUnpack = true;
  # Stripping would remove the program bun appends to its runtime.
  dontStrip = true;

  nativeBuildInputs = [ makeWrapper ] ++ lib.optionals stdenv.hostPlatform.isLinux [ wrapBuddy ];

  installPhase = ''
    runHook preInstall
    install -Dm755 $src $out/bin/claude
    runHook postInstall
  '';

  # One wrapProgram, so the process is still called claude rather than .claude-wrapped.
  #
  # DISABLE_AUTOUPDATER and DISABLE_INSTALLATION_CHECKS: Nix does the updating. Telemetry
  # and nonessential traffic stay on unless asked, because turning them off breaks
  # remote-control (https://github.com/numtide/llm-agents.nix/issues/2811).
  #
  # CLAUDE_CODE_CERT_STORE=system on Linux, so cacert removals apply rather than the roots
  # embedded in the binary. Not on darwin, whose system store lacks Apple's built-in roots.
  # bubblewrap and socat are what the sandbox needs on Linux.
  postFixup = ''
    wrapProgram $out/bin/claude \
      --argv0 claude \
      --set DISABLE_AUTOUPDATER 1 \
      --set DISABLE_INSTALLATION_CHECKS 1 \
      --set-default FORCE_AUTOUPDATE_PLUGINS 1 \
      --set-default DISABLE_NON_ESSENTIAL_MODEL_CALLS 1 \
      ${lib.optionalString disableTelemetry "--set DISABLE_TELEMETRY 1 --set CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC 1"} \
      ${lib.optionalString stdenv.hostPlatform.isLinux ''
        --set-default CLAUDE_CODE_CERT_STORE system \
        --prefix PATH : ${
          lib.makeBinPath [
            bubblewrap
            socat
          ]
        }
      ''}
  '';

  # bun links against /usr/lib/libicucore.A.dylib, which needs ICU data from /usr/share/icu
  # that the darwin sandbox hides, so the version check would fail there.
  __noChroot = stdenv.hostPlatform.isDarwin;

  doInstallCheck = true;
  nativeInstallCheckInputs = [ versionCheckHook ];

  meta = {
    description = "Agentic coding tool that lives in your terminal, understands your codebase, and helps you code faster";
    homepage = "https://claude.ai/code";
    changelog = "https://github.com/anthropics/claude-code/releases";
    license = lib.licenses.unfree;
    sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
    mainProgram = "claude";
    platforms = builtins.attrNames platforms;
  };
}
