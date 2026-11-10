# Mounts ccd-cli-fs at ~/.claude/remote/ccd-cli for the chosen users, as a systemd user
# service, so the Claude desktop app's SSH remote finds a runnable Claude Code for whatever
# version it asks for.
{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  inherit (lib) mkOption types;
  cfg = config.services.ccd-cli-fs;
  system = pkgs.stdenv.hostPlatform.system;
  flakePackages = self.packages.${system};
  versions = self.legacyPackages.${system}.claude-code-versions;
  newestFirst = lib.sort (a: b: lib.versionOlder b a) (lib.attrNames versions);
  fusermount = "${config.security.wrapperDir}/fusermount3";
  versionsDir = "%S/ccd-cli-fs/versions";
in
{
  options.services.ccd-cli-fs = {
    enable = lib.mkEnableOption "ccd-cli-fs, Nix-built Claude Code CLIs for the Claude desktop app's SSH remote";

    users = mkOption {
      type = types.nonEmptyListOf types.str;
      example = [ "philip" ];
      description = ''
        The users whose ~/.claude/remote/ccd-cli gets the filesystem. The service starts
        with the user's systemd manager, which an SSH login starts too late for the
        desktop app's first command; set `users.users.<name>.linger = true` so it is
        already running.
      '';
    };

    package = mkOption {
      type = types.package;
      default = flakePackages.ccd-cli-fs;
      defaultText = lib.literalExpression "claude-code-server.packages.\${system}.ccd-cli-fs";
      description = "The ccd-cli-fs package.";
    };

    clis = mkOption {
      type = types.listOf types.package;
      default = map (version: versions.${version}) (lib.take 3 newestFirst);
      defaultText = lib.literalMD "the newest 3 Claude Code versions this flake ships";
      example = lib.literalExpression ''
        [ claude-code-server.legacyPackages.''${system}.claude-code-versions."2.1.322" ]
      '';
      description = ''
        Claude Code packages to serve without building anything at runtime. Each is about
        330 MiB in the system closure. A version not here is answered by the newest of
        these (or of what the builder has built) while the builder builds it.
      '';
    };

    builder = {
      enable = mkOption {
        type = types.bool;
        default = true;
        description = ''
          Whether to build a version the desktop app asks for that is not in
          {option}`services.ccd-cli-fs.clis`, in the background, with Nix. Builds are kept
          as GC roots in ~/.local/state/ccd-cli-fs/versions; delete an entry there to let
          it be collected.
        '';
      };
      package = mkOption {
        type = types.package;
        default = flakePackages.ccd-cli-fs-builder;
        defaultText = lib.literalExpression "claude-code-server.packages.\${system}.ccd-cli-fs-builder";
        description = "The command ccd-cli-fs runs as `BUILDER VERSION`.";
      };
    };

    mountPoint = mkOption {
      type = types.str;
      default = "%h/.claude/remote/ccd-cli";
      description = ''
        Where to mount, with systemd specifiers (`%h` is the home directory). The desktop
        app's server is told `-cli-dir ~/.claude/remote/ccd-cli`.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    programs.fuse.enable = true;

    systemd.user.services.ccd-cli-fs = {
      description = "Claude Code CLIs for the Claude desktop app's SSH remote";
      wantedBy = [ "default.target" ];
      # Only for the chosen users: any one of these triggering conditions starts it.
      unitConfig.ConditionUser = map (user: "|${user}") cfg.users;
      environment = {
        FUSERMOUNT_PATH = fusermount;
        CCD_CLI_FS_VERSIONS_DIR = versionsDir;
      };
      serviceConfig = {
        Type = "exec";
        StateDirectory = "ccd-cli-fs";
        # mkdir -p may create ~/.claude/remote, which the desktop app keeps private.
        UMask = "0077";
        ExecStartPre = [
          # A daemon that died leaves its mount behind, answering ENOTCONN; clear it.
          "-${fusermount} -u -z ${cfg.mountPoint}"
          "${pkgs.coreutils}/bin/mkdir -p ${cfg.mountPoint} ${versionsDir}"
        ];
        ExecStart = lib.concatStringsSep " " (
          [
            (lib.getExe cfg.package)
            "--versions-dir"
            versionsDir
          ]
          ++ lib.optionals cfg.builder.enable [
            "--builder"
            (lib.getExe cfg.builder.package)
          ]
          ++ map (cli: "--cli ${cli.version}=${lib.getExe cli}") cfg.clis
          ++ [ cfg.mountPoint ]
        );
        Restart = "always";
        RestartSec = 1;
      };
    };
  };
}
