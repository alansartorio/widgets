# The month calendar the clock opens.
#
# A program, not a service: no unit, no socket, no state of its own. It is
# spawned by a click and exits when the window closes.
#
# It shares the holidays crate's files with kimai but not its config — the two
# read the same cache and the same override file at runtime, and neither Nix
# module writes either. That is what keeps them from fighting over the same
# `xdg.configFile` path the moment both are enabled.
{ flavour, self }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.calendar-gui;
  packages = self.packages.${pkgs.stdenv.hostPlatform.system};
in
{
  options.programs.calendar-gui = {
    enable = lib.mkEnableOption "the GTK4 month calendar with Argentine feriados";

    package = lib.mkOption {
      type = lib.types.package;
      default = packages.calendar-gui;
      defaultText = lib.literalMD "the flake's `calendar-gui`";
      description = "The `calendar-gui` binary the clock's click opens.";
    };
  };

  config = lib.mkIf cfg.enable (
    if flavour == "hm" then
      { home.packages = [ cfg.package ]; }
    else
      { environment.systemPackages = [ cfg.package ]; }
  );
}
