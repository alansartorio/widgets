# The shared body of both module flavours.
#
# `flavour` is "nixos" or "hm". The unit lands at the same attribute path either
# way — `systemd.user.services.timers-daemon` — but home-manager wants
# Unit/Service/Install sub-attrsets where NixOS wants description/wantedBy/
# serviceConfig, and the binaries go on PATH through different options. Nothing
# else differs, so the two wrappers are one line each.
{ flavour, self }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.timers;
  packages = self.packages.${pkgs.stdenv.hostPlatform.system};

  execStart = "${cfg.packages.daemon}/bin/timers-daemon";
  description = "Named countdown timers — deadline loop and control socket";

  # Bound to the graphical session rather than default.target on purpose: a
  # timer going off means a desktop notification, and there is nothing to notify
  # before a session — and its notification daemon — exists.
  target = "graphical-session.target";

  # StateDirectory= and RuntimeDirectory= on a *user* unit resolve to
  # $XDG_STATE_HOME/timers and $XDG_RUNTIME_DIR/timers, which is exactly where
  # the daemon looks by default. So no TIMERS_DATA_DIR or TIMERS_RUNTIME_DIR is
  # set here; those exist for running the daemon by hand against a scratch dir.
  serviceConfig = {
    ExecStart = execStart;
    Restart = "on-failure";
    RestartSec = 5;
    StateDirectory = "timers";
    RuntimeDirectory = "timers";
  };
in
{
  options.services.timers = {
    enable = lib.mkEnableOption "named countdown timers (daemon, CLI and GTK4 popup)";

    packages = lib.mkOption {
      description = ''
        The three binaries. They default to this flake's own, which is almost
        always what you want — the options exist so a host can substitute a
        build of its own without the module rebuilding the workspace.
      '';
      type = lib.types.submodule {
        options = {
          cli = lib.mkOption {
            type = lib.types.package;
            default = packages.timers;
            defaultText = lib.literalMD "the flake's `timers`";
            description = "The `timers` CLI, which the bar listens to.";
          };
          daemon = lib.mkOption {
            type = lib.types.package;
            default = packages.timers-daemon;
            defaultText = lib.literalMD "the flake's `timers-daemon`";
            description = "The `timers-daemon` binary the user service runs.";
          };
          gui = lib.mkOption {
            type = lib.types.package;
            default = packages.timers-gui;
            defaultText = lib.literalMD "the flake's `timers-gui`";
            description = "The `timers-gui` popup the bar's clicks open.";
          };
        };
      };
      default = { };
    };
  };

  config = lib.mkIf cfg.enable (
    # A user unit, not a system one, and that choice is load-bearing:
    # notify-rust needs DBUS_SESSION_BUS_ADDRESS, which a user unit inherits
    # from the session and a system unit has no way to reach. Moving this to
    # systemd.services would silently cost every notification.
    if flavour == "hm" then
      {
        home.packages = [
          cfg.packages.cli
          cfg.packages.gui
        ];
        systemd.user.services.timers-daemon = {
          Unit = {
            Description = description;
            PartOf = [ target ];
            After = [ target ];
          };
          Service = serviceConfig;
          Install.WantedBy = [ target ];
        };
      }
    else
      {
        environment.systemPackages = [
          cfg.packages.cli
          cfg.packages.gui
        ];
        systemd.user.services.timers-daemon = {
          inherit description serviceConfig;
          partOf = [ target ];
          after = [ target ];
          wantedBy = [ target ];
        };
      }
  );
}
