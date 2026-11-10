# The module on a VM, with stand-in CLIs (the real ones are unfree and large, and the
# desktop app's server is not packaged): versions resolve, the directory lists as empty
# and refuses writes, the versions directory is served, a killed daemon comes back, and
# missing versions are built by the template unit, once.
{ self, pkgs }:

let
  fakeClaude =
    version:
    pkgs.writeShellScriptBin "claude" ''echo "${version} (Claude Code)"''
    // {
      inherit version;
    };
  old = fakeClaude "2.1.300";
  new = fakeClaude "2.1.322";
  built = fakeClaude "2.1.400";
  onDemand = fakeClaude "2.1.500";
  # Logs what it is asked for. 2.1.500 it builds, 2.1.666 fails, and for anything else it
  # succeeds without leaving a build.
  fakeBuilder = pkgs.writeShellScriptBin "ccd-cli-fs-builder" ''
    echo "$1" >> "$HOME/builds.log"
    case $1 in
      2.1.500) ln -sfn ${onDemand} "$CCD_CLI_FS_VERSIONS_DIR/$1" ;;
      2.1.666) exit 1 ;;
    esac
  '';
in
pkgs.testers.runNixOSTest {
  name = "ccd-cli-fs";
  nodes.machine = {
    imports = [ self.nixosModules.default ];
    users.users.alice = {
      isNormalUser = true;
      uid = 1000;
      linger = true;
    };
    users.users.bob = {
      isNormalUser = true;
      uid = 1001;
    };
    services.ccd-cli-fs = {
      enable = true;
      users = [ "alice" ];
      clis = [
        old
        new
      ];
      builder.package = fakeBuilder;
    };
    system.extraDependencies = [
      built
      onDemand
    ];
  };
  testScript = ''
    def alice(command):
        return machine.succeed(f"su - alice -c {command!r}").strip()

    def alice_fails(command):
        machine.fail(f"su - alice -c {command!r}")

    def check_mount():
        assert alice("readlink ~/.claude/remote/ccd-cli/2.1.300") == "${pkgs.lib.getExe old}"
        assert alice("~/.claude/remote/ccd-cli/2.1.300 --version") == "2.1.300 (Claude Code)"
        # unknown and suffixed versions get the newest
        assert alice("~/.claude/remote/ccd-cli/9.9.9 --version") == "2.1.322 (Claude Code)"
        assert alice("~/.claude/remote/ccd-cli/2.1.301-abc --version") == "2.1.322 (Claude Code)"
        # what is not a version is not there
        alice_fails("test -e ~/.claude/remote/ccd-cli/2.1.322.zst")
        alice_fails("test -e ~/.claude/remote/ccd-cli/.lock")
        # nothing to list, so nothing to prune; nothing can be written
        assert alice("ls -A ~/.claude/remote/ccd-cli") == ""
        alice_fails("touch ~/.claude/remote/ccd-cli/x")
        alice_fails("rm ~/.claude/remote/ccd-cli/2.1.322")
        assert alice("stat -c '%U %a' ~/.claude/remote/ccd-cli") == "alice 755"

    machine.wait_for_unit("user@1000.service")
    machine.wait_for_unit("ccd-cli-fs.service", user="alice")
    check_mount()
    assert alice("stat -c %a ~/.claude/remote") == "700"

    with subtest("the versions directory is served"):
        alice("ln -s ${built} ~/.local/state/ccd-cli-fs/versions/2.1.400")
        assert alice("~/.claude/remote/ccd-cli/2.1.400 --version") == "2.1.400 (Claude Code)"
        assert alice("~/.claude/remote/ccd-cli/9.9.9 --version") == "2.1.400 (Claude Code)"
        alice("rm ~/.local/state/ccd-cli-fs/versions/2.1.400")

    with subtest("a killed daemon comes back"):
        machine.succeed("systemctl --user -M alice@ kill -s KILL ccd-cli-fs.service")
        machine.wait_until_succeeds("su - alice -c '~/.claude/remote/ccd-cli/2.1.300 --version'")
        check_mount()

    with subtest("a missing version is built by the template unit"):
        assert alice("~/.claude/remote/ccd-cli/2.1.500 --version") == "2.1.322 (Claude Code)"
        machine.wait_until_succeeds(
            "su - alice -c '~/.claude/remote/ccd-cli/2.1.500 --version' | grep -Fx '2.1.500 (Claude Code)'"
        )
        alice("rm ~/.local/state/ccd-cli-fs/versions/2.1.500")

    with subtest("a suffixed name builds its release"):
        alice("readlink ~/.claude/remote/ccd-cli/2.1.501-abc")
        machine.wait_until_succeeds("grep -Fx 2.1.501 ~alice/builds.log")
        machine.fail("grep -F 2.1.501-abc ~alice/builds.log")

    with subtest("a build that fails or leaves nothing is not run by every lookup"):
        for _ in range(4):
            alice("readlink ~/.claude/remote/ccd-cli/2.1.600 ~/.claude/remote/ccd-cli/2.1.666")
            machine.sleep(2)
        assert alice("grep -cFx 2.1.600 ~/builds.log") == "1"
        assert alice("grep -cFx 2.1.666 ~/builds.log") == "1"

    with subtest("other users get nothing"):
        machine.succeed("loginctl enable-linger bob")
        machine.wait_for_unit("user@1001.service")
        machine.wait_for_unit("default.target", user="bob")
        machine.fail("su - bob -c 'test -e ~/.claude/remote/ccd-cli'")
  '';
}
