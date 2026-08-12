# Everything in this flake, as NixOS modules, in one import.
#
# Importing this declares the options and turns nothing on; each service still
# needs its own `enable`.
#
# The timers, kimai and calendar flavours here are for a machine with no
# home-manager: they define the same units and files system-wide, which applies
# them to every user on the box. On a machine that *does* have home-manager,
# import `homeManagerModules.default` for those three and take only
# `nixosModules.claude-usage` from here — that one has no home-manager flavour
# by design, since the whole point of it is a daemon running as its own user.
{ self }:
{
  imports = [
    (import ./modules/claude-usage.nix {
      flavour = "nixos";
      inherit self;
    })
    (import ./modules/timers.nix {
      flavour = "nixos";
      inherit self;
    })
    (import ./modules/kimai.nix {
      flavour = "nixos";
      inherit self;
    })
    (import ./modules/calendar.nix {
      flavour = "nixos";
      inherit self;
    })
  ];
}
