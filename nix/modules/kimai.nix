# The kimai integration: the CLI the bar listens to, the popup its right click
# opens, and the config file both read.
#
# **No systemd unit at all**, unlike every other module in this repo, and that
# is not an omission. There is no daemon: nothing to keep between polls that
# Kimai is not already the authority on, nothing to notify, no state to own. So
# this module's entire job is the config file and PATH.
#
# It also seeds — once — the holiday override file, which is the one file here
# Nix deliberately does *not* manage. See `holidays.seedOverrides` below.
{ flavour, self }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.kimai;
  packages = self.packages.${pkgs.stdenv.hostPlatform.system};

  # The same file `holidays::overrides::TEMPLATE` include_str!s, so the seed
  # written here and the one the CLI would write cannot document different
  # spellings of `kind`.
  seedFile = ../../crates/shared/holidays/overrides.template.toml;

  configFile = (pkgs.formats.toml { }).generate "kimai-config.toml" (
    {
      base_url = cfg.url;
      token_file = toString cfg.tokenFile;
      daily_target_hours = cfg.dailyTargetHours;
      week_start = cfg.weekStart;
      workdays = cfg.workdays;
      bridges_are_working = cfg.bridgesAreWorking;
      poll_seconds = cfg.pollSeconds;
      span_poll_seconds = cfg.spanPollSeconds;
      holidays_max_age_days = cfg.holidays.maxAgeDays;
    }
  );
in
{
  options.services.kimai = {
    enable = lib.mkEnableOption "the Kimai integration (bar CLI and GTK4 popup)";

    packages = lib.mkOption {
      description = "The two binaries. They default to this flake's own.";
      type = lib.types.submodule {
        options = {
          cli = lib.mkOption {
            type = lib.types.package;
            default = packages.kimai;
            defaultText = lib.literalMD "the flake's `kimai`";
            description = "The `kimai` CLI, which the bar listens to.";
          };
          gui = lib.mkOption {
            type = lib.types.package;
            default = packages.kimai-gui;
            defaultText = lib.literalMD "the flake's `kimai-gui`";
            description = "The popup the bar's right click opens.";
          };
        };
      };
      default = { };
    };

    url = lib.mkOption {
      type = lib.types.str;
      example = "https://kimai.example.com";
      description = "Base URL of the Kimai instance.";
    };

    tokenFile = lib.mkOption {
      type = lib.types.path;
      example = "/run/secrets/kimai_token";
      description = ''
        A file holding a Kimai personal API token (profile → API access).

        The *path* is what goes into the config file, never the token: it is
        read at runtime, so nothing secret lands in the nix store or in
        ~/.config. A sops secret's path is the intended value.
      '';
    };

    dailyTargetHours = lib.mkOption {
      type = lib.types.either lib.types.int lib.types.float;
      default = 8;
      description = "Hours a working day is expected to be.";
    };

    weekStart = lib.mkOption {
      type = lib.types.enum [
        "monday"
        "sunday"
      ];
      default = "monday";
      description = "Which day the week's total starts counting from.";
    };

    workdays = lib.mkOption {
      type = lib.types.listOf (
        lib.types.enum [
          "mon"
          "tue"
          "wed"
          "thu"
          "fri"
          "sat"
          "sun"
        ]
      );
      default = [
        "mon"
        "tue"
        "wed"
        "thu"
        "fri"
      ];
      description = "Which weekdays carry the daily target.";
    };

    bridgesAreWorking = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Whether a *puente turístico* counts as a working day.

        False by default because "no laborable" is what the decree says until an
        employer decides otherwise. Individual dates can be flipped either way
        in the override file without a rebuild, which is usually the better
        tool — this is for a workplace where the answer is the same every time.
      '';
    };

    pollSeconds = lib.mkOption {
      type = lib.types.ints.positive;
      default = 60;
      description = "How often today's numbers are refreshed.";
    };

    spanPollSeconds = lib.mkOption {
      type = lib.types.ints.positive;
      default = 900;
      description = ''
        How often the surrounding week-and-month span is refreshed. Much rarer
        than `pollSeconds` on purpose: yesterday does not change, and today's
        poll is what keeps the week and month totals live.
      '';
    };

    holidays = {
      maxAgeDays = lib.mkOption {
        type = lib.types.ints.positive;
        default = 7;
        description = "How stale the feriado cache may get before a refresh.";
      };

      seedOverrides = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = ''
          Write a commented `~/.config/holidays/overrides.toml` if none exists.

          Seeded, not managed, and the distinction is the whole point. Deciding
          that today is not being worked is something you do at 08:55 by editing
          a file; an `xdg.configFile` would make it a read-only symlink into the
          store, and home-manager would clobber any replacement on the next
          switch. So this writes it once, if it is absent, and never touches it
          again.

          An absent file works fine — it means no overrides. The reason to seed
          one anyway is discoverability: nobody reads a man page for a file they
          are about to edit, but they do read the file, and the template is
          where the four `kind` values are documented.
        '';
      };
    };
  };

  config = lib.mkIf cfg.enable (
    let
      common = {
        assertions = [
          {
            assertion = cfg.url != "";
            message = "services.kimai.enable requires services.kimai.url.";
          }
        ];
      };
    in
    if flavour == "hm" then
      common
      // {
        home.packages = [
          cfg.packages.cli
          cfg.packages.gui
        ];
        xdg.configFile."kimai/config.toml".source = configFile;

        home.activation = lib.mkIf cfg.holidays.seedOverrides {
          seedHolidayOverrides = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
            overrides="${config.xdg.configHome}/holidays/overrides.toml"
            if [ ! -e "$overrides" ]; then
              run mkdir -p "$(dirname "$overrides")"
              # install -m, not cp: a file copied out of the store inherits 0444
              # and could not be edited afterwards, which is the exact failure
              # this whole arrangement exists to avoid.
              run install -m 0644 ${seedFile} "$overrides"
            fi
          '';
        };
      }
    else
      common
      // {
        environment.systemPackages = [
          cfg.packages.cli
          cfg.packages.gui
        ];
        # $XDG_CONFIG_DIRS includes /etc/xdg, which is where the CLI looks after
        # the user's own config dir — so this is the system-wide fallback rather
        # than something that overrides a per-user file.
        environment.etc."xdg/kimai/config.toml".source = configFile;
      }
  );
}
