# Everything in this flake that can run as you, in one import.
#
# Importing this declares the options and turns nothing on; each service still
# needs its own `enable`. So a host that wants only the calendar pays for the
# others in evaluation time and nothing else, which is cheap enough that
# picking modules individually is rarely worth the extra import lines — the
# per-service modules stay exported for the hosts where it is.
#
# claude-usage is not here: it is a system service with a user of its own, and
# lives in `nixosModules` alone.
{ self }:
{
  imports = [
    (import ./modules/timers.nix {
      flavour = "hm";
      inherit self;
    })
    (import ./modules/kimai.nix {
      flavour = "hm";
      inherit self;
    })
    (import ./modules/calendar.nix {
      flavour = "hm";
      inherit self;
    })
    (import ./modules/hass.nix {
      flavour = "hm";
      inherit self;
    })
  ];
}
