# claude-code-server

Nix-built Claude Code CLIs for the Claude desktop app's SSH remote, so a NixOS host works as
a remote without nix-ld.

## The problem

When the desktop app connects to a host over SSH, it installs a helper there
(`~/.claude/remote/srv/<commit>/server`, which calls itself `claude-ssh`) and runs

```
server -install -cli-dir ~/.claude/remote/ccd-cli -cli-version <version> -cli-url … -cli-checksum …
```

If `ccd-cli/<version>` is missing, the server downloads Anthropic's generic glibc build
there. Then it probes that file by running `<version> --version`. On NixOS
`/lib64/ld-linux-x86-64.so.2` is a stub, so the probe fails and the install is reported as
"not runnable".

## How this solves it

`ccd-cli-fs` is a FUSE filesystem mounted at `~/.claude/remote/ccd-cli`. Looking up any
version name finds a symlink to a Nix-built Claude Code:

- the build of that version, if there is one;
- for a suffixed name like `2.1.295-…`, the build of `2.1.295`;
- otherwise the newest build stands in, and a build of the requested version starts in the
  background, as the systemd user unit `ccd-cli-fs-build@<version>.service`. Lookups never
  wait for it, because the probe has a time limit.

The server sees the name exists, so it skips the download, and the probe runs the Nix
build. Names that are not versions (`.zst` files, lock files) don't exist. The directory
lists as empty, so the server's pruning (`-cli-keep`) finds nothing to delete. The mount is
read-only.

Builds come from two places:

- `--cli VERSION=PATH`: store paths fixed at build time. The module passes the newest 3
  versions this flake ships, so those need no work at runtime.
- `--versions-dir DIR`: out-links named by version. The directory is read again on every
  lookup. `ccd-cli-fs-builder` fills it, and so can you:

  ```
  nix build --out-link ~/.local/state/ccd-cli-fs/versions/2.1.310 .#claude-code_2_1_310
  ```

  Each entry is a GC root. Delete it to let Nix collect the build.

`ccd-cli-fs-builder VERSION` gets the hash from `versions.json`. For a version newer than
the flake, which is common because the app tends to be newer than your lock file, it gets
the hash from the release's `manifest.json` on downloads.claude.ai. The unit's start limit
keeps a build that failed, or that left nothing servable, from being retried for 10 minutes.
Follow one with `journalctl --user -u 'ccd-cli-fs-build@*'`, or start one yourself:

```
systemctl --user start ccd-cli-fs-build@2.1.310
```

## Use on NixOS

```nix
{
  inputs.claude-code-server.url = "github:philiptaron/claude-code-server";
  inputs.claude-code-server.inputs.nixpkgs.follows = "nixpkgs";

  # in the host's modules:
  imports = [ inputs.claude-code-server.nixosModules.default ];
  services.ccd-cli-fs = {
    enable = true;
    users = [ "philip" ];
  };
  # The user service has to be running already when the desktop app's first SSH command
  # arrives.
  users.users.philip.linger = true;
}
```

Options:

- `clis`: which packages to serve without building. The default is the newest 3, about
  330 MiB each.
- `builder.enable`: whether to build missing versions. Defaults to true.
- `builder.retryAfter`: seconds before a version is built again. Defaults to 600.
- `mountPoint`: where to mount.

Watch it with `journalctl --user -u ccd-cli-fs -f`. Each lookup is logged with what
answered it.

### First run

Before connecting from the desktop app for the first time, run `claude` once interactively
on the host, logged in as that user. A CLI that has never been run shows its setup wizard,
and the app's non-interactive start fails silently on it
([anthropics/claude-code#46845](https://github.com/anthropics/claude-code/issues/46845)).

### If the daemon dies

Until systemd restarts it, about a second later, the mount answers ENOTCONN. During that
window the server fails with `mkdir cli dir: … file exists`, safely: it downloads nothing.
Before each start, the service lazily unmounts whatever mount is left behind.

## Packages

The flake's only input is nixpkgs. It ships the 20 newest Claude Code releases for
x86_64-linux, aarch64-linux and aarch64-darwin:

- `packages.<system>.claude-code_2_1_322` and so on, with `claude-code` for the newest;
- `legacyPackages.<system>.claude-code-versions."2.1.322"`, and the same set through
  `overlays.default`, which also adds `ccd-cli-fs` and `ccd-cli-fs-builder`.

The packaging follows [numtide/llm-agents.nix](https://github.com/numtide/llm-agents.nix) but
doesn't depend on it:

- it uses the uncompressed binary from the release bucket;
- [wrap-buddy](https://github.com/Mic92/wrap-buddy) makes the binary runnable instead of
  autoPatchelf;
- a single `wrapProgram` sets `DISABLE_AUTOUPDATER` and `DISABLE_INSTALLATION_CHECKS`;
- telemetry is left on, because turning it off breaks remote-control. Override with
  `disableTelemetry = true` to turn it off anyway.

Claude Code is unfree. The flake's own package set allows it. If you use the overlay, your
nixpkgs has to allow it.

## Updates

- **Claude Code**: `.github/workflows/update.yml` runs `packages/claude-code/update.py`
  hourly. GitHub can't trigger a workflow from another repository's releases: release
  events and webhooks only go to the repository that owns them, so polling is the only
  option. The script follows downloads.claude.ai's `latest` and keeps nothing above it,
  because Anthropic yanks a bad release by pointing `latest` back down. It takes the list
  of releases below `latest` from the npm registry: GitHub's releases skip some versions
  that downloads.claude.ai has (2.1.315, for one), and the bucket can't be listed. The job
  builds each added version, then pushes `versions.json`.
- **nixpkgs, Cargo, Actions**: Dependabot. Its `nix` ecosystem updates `flake.lock`. The
  crate builds from `Cargo.lock` without a `cargoHash`, so Cargo update pull requests build
  without any hash changes.

## Development

```
nix develop
cargo test
nix flake check     # also runs the module's NixOS VM test
nix build .#claude-code
```

To try a change against the real server, mount a scratch directory and point the server at
it:

```
ccd-cli-fs --cli 2.1.322=$(nix build --no-link --print-out-paths .#claude-code)/bin/claude /tmp/cli &
~/.claude/remote/srv/*/server -install -cli-dir /tmp/cli -cli-version 2.1.999
~/.claude/remote/srv/*/server -probe-cli /tmp/cli/2.1.999
```
