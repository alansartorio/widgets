# The Home Assistant light pill: one user service and the config file it reads.
#
# This replaces a `writeShellApplication` that lived in the nixos-config and did
# four things by hand — read the token out of /run/secrets, export it, `killall`
# the previous copy, and pipe `socat -u UNIX-RECV:/tmp/hass-light.sock` into the
# daemon's stdin. All four are gone: the daemon binds its own sockets, reads the
# token off disk itself, and being the only copy of itself is what a systemd
# unit is for. The compositor no longer spawns anything at login.
#
# The token is a *path* here, as in `kimai`: `HASS_TOKEN` in a process
# environment is readable by anything else running as the same user, and an
# exported secret outlives the reason it was exported.
{ flavour, self }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.hass;
  packages = self.packages.${pkgs.stdenv.hostPlatform.system};

  configFile = (pkgs.formats.toml { }).generate "hass-config.toml" (
    {
      base_url = cfg.url;
      token_file = toString cfg.tokenFile;
      entity = cfg.entity;
      step_percent = cfg.stepPercent;
      min_brightness_percent = cfg.minBrightnessPercent;
      settle_poll_ms = cfg.settlePollMs;
      settle_timeout_ms = cfg.settleTimeoutMs;
      request_timeout_seconds = cfg.requestTimeoutSeconds;
    }
    # Both default to $XDG_RUNTIME_DIR/hass/… inside the daemon, and the file
    # says nothing at all when nobody overrode them — writing the resolved path
    # here would bake this user's uid into a config a second user reads.
    // lib.optionalAttrs (cfg.commandSocket != null) { command_socket = toString cfg.commandSocket; }
    // lib.optionalAttrs (cfg.stateSocket != null) { state_socket = toString cfg.stateSocket; }
  );

  description = "Home Assistant light pill — the bar's brightness and colour-temperature control";

  # Bound to the graphical session: the only thing that ever sends this a
  # command is a bar that does not exist before one.
  target = "graphical-session.target";

  serviceConfig = {
    ExecStart = "${cfg.package}/bin/hass-daemon";
    Restart = "always";
    # Longer than the default because the usual reason to fail at start is a
    # Home Assistant that is not answering yet — a suspend/resume, a router
    # rebooting — and a five-second retry loop against an absent host is just
    # noise in the journal.
    RestartSec = 15;
    # Resolves to $XDG_RUNTIME_DIR/hass on a user unit, which is where the
    # daemon puts both sockets unless told otherwise.
    RuntimeDirectory = "hass";
  };
in
{
  options.services.hass = {
    enable = lib.mkEnableOption "the Home Assistant light pill (a user daemon and two sockets)";

    package = lib.mkOption {
      type = lib.types.package;
      default = packages.hass-daemon;
      defaultText = lib.literalMD "the flake's `hass-daemon`";
      description = "The `hass-daemon` binary the user service runs.";
    };

    url = lib.mkOption {
      type = lib.types.str;
      example = "http://homeassistant.local:8123";
      description = "Base URL of the Home Assistant instance.";
    };

    tokenFile = lib.mkOption {
      type = lib.types.path;
      example = "/run/secrets/hass_token";
      description = ''
        A file holding a long-lived access token (profile → security → long-lived
        access tokens).

        The *path* is what goes into the config file, never the token: it is read
        at runtime, so nothing secret lands in the nix store, in ~/.config or in
        a process environment. A sops secret's path is the intended value.
      '';
    };

    entity = lib.mkOption {
      type = lib.types.str;
      example = "light.desk_lamp";
      description = ''
        The light entity the pill drives.

        Required, and with no default on purpose: the binary this module replaces
        had one particular bedroom lamp compiled into it.
      '';
    };

    commandSocket = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      defaultText = lib.literalMD "`$XDG_RUNTIME_DIR/hass/command.sock`";
      example = "/tmp/hass-light.sock";
      description = ''
        Where the daemon listens for `increase`, `decrease`, `toggle_mode` and
        `toggle_state`, one word per datagram.

        The default lives under the runtime directory rather than in `/tmp`,
        which is a change from the old launcher: a world-writable directory lets
        anything on the box take the path first and collect the commands. Set
        this to the old path if some eww config you would rather not touch names
        it.
      '';
    };

    stateSocket = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      defaultText = lib.literalMD "`$XDG_RUNTIME_DIR/hass/state.sock`";
      example = "/tmp/hass-light-eww-output.sock";
      description = ''
        Where the daemon publishes the widget's state, one JSON object per line,
        to every connected reader. This is what the bar's `deflisten` connects to.
      '';
    };

    stepPercent = lib.mkOption {
      type = lib.types.either lib.types.int lib.types.float;
      default = 10;
      description = "How far one scroll notch moves brightness or colour temperature.";
    };

    minBrightnessPercent = lib.mkOption {
      type = lib.types.either lib.types.int lib.types.float;
      default = 0.39;
      description = ''
        The floor on brightness, in percent. 0.39 is 1/255 — the dimmest a lamp
        can be while still being on, which is what keeps scrolling all the way
        down from turning it off behind the widget's back.
      '';
    };

    settlePollMs = lib.mkOption {
      type = lib.types.ints.positive;
      default = 100;
      description = ''
        How often to re-read a lamp that is on but has not reported a brightness
        yet. The pill shows `loading` until it does.
      '';
    };

    settleTimeoutMs = lib.mkOption {
      type = lib.types.ints.positive;
      default = 10000;
      description = ''
        How long to keep asking, re-sending `turn_on` while the lamp reads as
        off, before the pill falls back to whatever the lamp last reported.

        The binary this replaces had no ceiling: ten requests a second, forever,
        against a bulb that never answered.
      '';
    };

    requestTimeoutSeconds = lib.mkOption {
      type = lib.types.ints.positive;
      default = 10;
      description = "How long any one HTTP request may take before it is given up on.";
    };
  };

  config = lib.mkIf cfg.enable (
    let
      common = {
        assertions = [
          {
            assertion = cfg.url != "";
            message = "services.hass.enable requires services.hass.url.";
          }
          {
            assertion = cfg.entity != "";
            message = "services.hass.enable requires services.hass.entity, e.g. \"light.desk_lamp\".";
          }
        ];
      };
    in
    # A user unit rather than a system one, for the same reason as timers: the
    # sockets belong to a session's bar, and $XDG_RUNTIME_DIR is where a session
    # keeps things only it should reach.
    if flavour == "hm" then
      common
      // {
        home.packages = [ cfg.package ];
        xdg.configFile."hass/config.toml".source = configFile;

        systemd.user.services.hass-daemon = {
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
      common
      // {
        environment.systemPackages = [ cfg.package ];
        # $XDG_CONFIG_DIRS includes /etc/xdg, which is where the daemon looks
        # after the user's own config dir — the system-wide fallback rather than
        # something that overrides a per-user file.
        environment.etc."xdg/hass/config.toml".source = configFile;

        systemd.user.services.hass-daemon = {
          inherit description serviceConfig;
          partOf = [ target ];
          after = [ target ];
          wantedBy = [ target ];
        };
      }
  );
}
