#!/usr/bin/env python3
"""Keep versions.json at the newest Claude Code releases.

The newest release is what downloads.claude.ai/claude-code-releases/latest names. Anthropic
yanks a bad release by pointing `latest` back down, so nothing above it is kept. The
releases below it come from the npm registry's list of @anthropic-ai/claude-code versions:
GitHub's release list skips some versions that downloads.claude.ai has (2.1.315, for one),
and the bucket cannot be listed. A version is kept only if its manifest.json exists and
has every platform the package builds for; the hashes come from that manifest.

Hashes of versions already in versions.json are kept without asking again, so a run asks
only about new versions. Prints the versions added and removed, one per line, as
`+VERSION` or `-VERSION`.

Usage: update.py [--keep N] [VERSIONS_JSON]
"""

import argparse
import base64
import json
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path

RELEASES = "https://downloads.claude.ai/claude-code-releases"
NPM = "https://registry.npmjs.org/@anthropic-ai/claude-code"
# nix system -> platform name in the manifest and download URL
PLATFORMS = {
    "x86_64-linux": "linux-x64",
    "aarch64-linux": "linux-arm64",
    "aarch64-darwin": "darwin-arm64",
}
VERSION = re.compile(r"([0-9]+)\.([0-9]+)\.([0-9]+)")
USER_AGENT = "claude-code-server-updater"


def fetch(url):
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read()


def key(version):
    return tuple(int(n) for n in VERSION.fullmatch(version).groups())


def sri(hex_digest):
    if not re.fullmatch(r"[0-9a-f]{64}", hex_digest):
        raise ValueError(f"not a sha256 digest: {hex_digest!r}")
    return "sha256-" + base64.b64encode(bytes.fromhex(hex_digest)).decode()


def hashes_of(version):
    """The SRI hash of each platform's binary, or None if the release is not there."""
    try:
        manifest = json.loads(fetch(f"{RELEASES}/{version}/manifest.json"))
    except urllib.error.HTTPError as error:
        if error.code in (403, 404):
            return None
        raise
    if manifest.get("version") != version:
        raise ValueError(f"manifest of {version} is for {manifest.get('version')!r}")
    entries = manifest.get("platforms", {})
    hashes = {}
    for system, platform in PLATFORMS.items():
        entry = entries.get(platform)
        if entry is None or entry.get("binary") != "claude":
            print(f"{version}: no {platform} binary, skipping", file=sys.stderr)
            return None
        hashes[system] = sri(entry["checksum"])
    return hashes


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--keep", type=int, default=20)
    parser.add_argument(
        "versions_json", nargs="?", type=Path, default=Path(__file__).parent / "versions.json"
    )
    args = parser.parse_args()

    old = json.loads(args.versions_json.read_text()) if args.versions_json.exists() else {}

    latest = fetch(f"{RELEASES}/latest").decode().strip()
    if not VERSION.fullmatch(latest):
        raise ValueError(f"latest is not a version: {latest!r}")
    published = json.loads(fetch(NPM))["versions"].keys()
    candidates = {v for v in published if VERSION.fullmatch(v)} | set(old) | {latest}
    candidates = sorted((v for v in candidates if key(v) <= key(latest)), key=key, reverse=True)

    new = {}
    for version in candidates:
        if len(new) == args.keep:
            break
        hashes = old.get(version) or hashes_of(version)
        if hashes is not None:
            new[version] = hashes

    if latest not in new:
        raise ValueError(f"latest release {latest} has no usable manifest")

    args.versions_json.write_text(json.dumps(new, indent=2) + "\n")
    for version in sorted(set(new) - set(old), key=key):
        print(f"+{version}")
    for version in sorted(set(old) - set(new), key=key):
        print(f"-{version}")


if __name__ == "__main__":
    main()
