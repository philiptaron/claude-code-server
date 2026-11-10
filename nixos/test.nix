# The module on a VM, with stand-in CLIs (the real ones are unfree and large, and the
# desktop app's server is not packaged): versions resolve, the directory lists as empty
# and refuses writes, the versions directory is served, and a killed daemon comes back.
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
      builder.enable = false;
    };
    system.extraDependencies = [ built ];
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

    with subtest("other users get nothing"):
        machine.succeed("loginctl enable-linger bob")
        machine.wait_for_unit("user@1001.service")
        machine.wait_for_unit("default.target", user="bob")
        machine.fail("su - bob -c 'test -e ~/.claude/remote/ccd-cli'")
  '';
}
