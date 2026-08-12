# NixOS module for the claude-usage tracker.
#
# The odd one out in this repo: a *system* service with a user of its own, where
# timers, kimai and the calendar all run as you. That is deliberate — the daemon
# holds an API token, and keeping it in /var/lib behind a system user means the
# desktop session cannot read it even by accident.
#
# NixOS only, for the same reason. There is no home-manager flavour because
# there is nothing sensible for one to do: a per-user copy of this would put the
# token back in the home directory the system flavour exists to keep it out of.
#
# Everything machine-specific — the ssh key, known_hosts, the exact refresh
# command — is out of scope here and belongs in the host's config. This module
# only knows that *some* command produces a token.
{ flavour, self }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.claude-usage;
  packages = self.packages.${pkgs.stdenv.hostPlatform.system};
  dataDir = "/var/lib/claude-usage";
  user = "claude-usage";
  group = "claude-usage";
  bins = cfg.packages;
in
{
  options.services.claude-usage = {
    enable = lib.mkEnableOption "Claude Code usage tracker (daemon + CLI + GUI)";

    packages = lib.mkOption {
      description = "The three binaries. They default to this flake's own.";
      type = lib.types.submodule {
        options = {
          cli = lib.mkOption {
            type = lib.types.package;
            default = packages.claude-usage;
            defaultText = lib.literalMD "the flake's `claude-usage`";
            description = "The `claude-usage` CLI, which the bar listens to.";
          };
          daemon = lib.mkOption {
            type = lib.types.package;
            default = packages.claude-usage-daemon;
            defaultText = lib.literalMD "the flake's `claude-usage-daemon`";
            description = "The daemon the system service runs.";
          };
          gui = lib.mkOption {
            type = lib.types.package;
            default = packages.claude-usage-gui;
            defaultText = lib.literalMD "the flake's `claude-usage-gui`";
            description = "The chart popup the bar's click opens.";
          };
        };
      };
      default = { };
    };

    socketGroupMembers = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      example = [ "alice" ];
      description = ''
        Users to add to the `claude-usage` group, which is what the 0660
        control socket is readable by. Without at least one, the daemon runs and
        nothing can ask it anything.
      '';
    };

    tokenRefreshCommand = lib.mkOption {
      type = lib.types.str;
      default = "";
      example = "ssh -o BatchMode=yes -i /var/lib/claude-usage/.ssh/id_ed25519 workstation.local cat .claude/.credentials.json";
      description = ''
        Shell command whose stdout is a credentials JSON (or bare access
        token). Run by the daemon to refresh its token when the endpoint
        rejects it; the result is persisted to ${dataDir}/credentials.json.

        Typically an ssh that cats a remote machine's
        ~/.claude/.credentials.json. The ssh key and known_hosts for that are
        machine-specific and must be set up out of band — this module only runs
        whatever string you give it.
      '';
    };

    pollInterval = lib.mkOption {
      type = lib.types.ints.positive;
      default = 300;
      example = 60;
      description = ''
        How often the daemon polls the endpoint, in seconds.

        Not a systemd time span: the daemon owns its own timer (it sleeps on a
        condvar so `PollNow` can cut the sleep short), so this is passed to it
        as `CLAUDE_USAGE_POLL_INTERVAL` and never reaches systemd.
      '';
    };
  };

  config = lib.mkIf cfg.enable (
    assert lib.assertMsg (flavour == "nixos")
      "services.claude-usage has no home-manager flavour: the daemon runs as its own system user, which is the point of it.";
    {
      users.users.${user} = {
        isSystemUser = true;
        inherit group;
        # The daemon sets HOME from this; ssh and other tools read it for
        # ~/.something lookups. The state dir is the natural place.
        home = dataDir;
        createHome = false;
      };
      # `members` is who can reach the control socket, which the daemon creates
      # 0660. Without at least one the daemon runs and nothing can ask it
      # anything.
      users.groups.${group}.members = cfg.socketGroupMembers;

      systemd.services.claude-usage-daemon = {
        description = "Claude Code usage tracker — polls the endpoint and serves a control socket";
        wantedBy = [ "multi-user.target" ];
        after = [ "network-online.target" ];
        wants = [ "network-online.target" ];

        serviceConfig = {
          User = user;
          Group = group;
          StateDirectory = "claude-usage";
          StateDirectoryMode = "0750";
          RuntimeDirectory = "claude-usage";
          RuntimeDirectoryMode = "0755";
          ExecStart = "${bins.daemon}/bin/claude-usage-daemon";
          Restart = "on-failure";
          RestartSec = 10;
        };

        # The daemon runs the refresh command via `sh -c "$cmd"`, so `sh` must
        # be on the service PATH. A dedicated system user gets a minimal PATH
        # that doesn't include bash, so add it explicitly.
        path = [ pkgs.bash ];

        # Keep the token command out of argv so it's not world-readable in ps.
        # Use the `environment` attrset (not `serviceConfig.Environment` as a
        # raw string): a value with spaces, like "ssh -o ... host cat ...",
        # must be quoted for systemd, and NixOS's `environment` generator does
        # that correctly — `serviceConfig.Environment = "KEY=val with spaces"`
        # is emitted unquoted and gets truncated at the first space.
        environment = {
          CLAUDE_USAGE_REFRESH_COMMAND = cfg.tokenRefreshCommand;
          CLAUDE_USAGE_POLL_INTERVAL = toString cfg.pollInterval;
        };
      };

      environment.systemPackages = [
        bins.cli
        bins.gui
      ];
    }
  );
}
