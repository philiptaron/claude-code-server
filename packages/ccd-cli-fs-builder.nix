# The command run as `ccd-cli-fs-builder VERSION` when a version is missing, by the unit
# ccd-cli-fs-build@VERSION.service that ccd-cli-fs starts (see nixos/module.nix). It
# builds that Claude Code with Nix and leaves the out-link VERSION in
# $CCD_CLI_FS_VERSIONS_DIR, where ccd-cli-fs finds it on the next lookup. The hash comes
# from versions.json when the version is there, else from the release's manifest.json.
{
  lib,
  stdenv,
  writeShellApplication,
  coreutils,
  curl,
  jq,
  nix,
  # The nixpkgs to build with, as a path.
  nixpkgsPath,
}:

let
  platform =
    {
      x86_64-linux = "linux-x64";
      aarch64-linux = "linux-arm64";
    }
    .${stdenv.hostPlatform.system};
in
writeShellApplication {
  name = "ccd-cli-fs-builder";
  runtimeInputs = [
    coreutils
    curl
    jq
    nix
  ];
  text = ''
    version=''${1:?usage: ccd-cli-fs-builder VERSION}
    dir=''${CCD_CLI_FS_VERSIONS_DIR:?CCD_CLI_FS_VERSIONS_DIR is not set}
    system=${stdenv.hostPlatform.system}

    if [[ ! $version =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.+_-]+)?$ ]]; then
      echo "ccd-cli-fs-builder: $version is not a version" >&2
      exit 1
    fi

    hash=$(jq -r --arg v "$version" --arg s "$system" '.[$v][$s] // empty' ${./.}/claude-code/versions.json)
    if [[ -z $hash ]]; then
      url=https://downloads.claude.ai/claude-code-releases/$version/manifest.json
      echo "ccd-cli-fs-builder: $version is not in versions.json, asking $url" >&2
      checksum=$(curl -fsSL --max-time 60 "$url" | jq -r --arg p ${platform} '.platforms[$p].checksum // empty')
      if [[ ! $checksum =~ ^[0-9a-f]{64}$ ]]; then
        echo "ccd-cli-fs-builder: no ${platform} checksum for $version" >&2
        exit 1
      fi
      hash=$(nix --extra-experimental-features nix-command hash convert --hash-algo sha256 --to sri "$checksum")
    fi

    mkdir -p "$dir"
    nix --extra-experimental-features nix-command build --impure \
      --file ${./.}/claude-code/build.nix \
      --argstr nixpkgs ${nixpkgsPath} \
      --argstr system "$system" \
      --argstr version "$version" \
      --argstr hash "$hash" \
      --out-link "$dir/$version"
    echo "ccd-cli-fs-builder: built $version as $(readlink -f "$dir/$version")" >&2
  '';
  meta.platforms = lib.platforms.linux;
}
