# widgets

The tools behind the eww bar's pills, plus the calendar the clock opens. One
cargo workspace, one flake, four integrations over four shared crates.

```
crates/
  shared/
    ipc/        length-prefixed bincode framing, Endpoint/Subscription, bind()
    paths/      XDG (user services) and /var/lib (claude-usage) directories
    gtkui/      layer-shell placement, single-instance, the Nord palette
    holidays/   Argentine feriados, the override file, working-day arithmetic
  claude-usage/ core + daemon + cli + gui   — API usage, a system service
  timers/       core + daemon + cli + gui   — named countdown timers
  kimai/        api + core + cli + gui      — time tracking, no daemon
  calendar/     gui                         — the month, with feriados
nix/
  mkbin.nix     one workspace build, one thin derivation per binary
  modules/      one per integration, each `{ flavour }` for NixOS and HM
  hm.nix nixos.nix   the aggregate `default` modules
```

## Why one repo

`claude-usage` and `timers` used to be a repo each, and their `proto.rs` framing
was byte-identical apart from one constant; `client.rs` differed only in doc
comments; `paths.rs` and `db.rs` were near-copies. Two of everything to update,
GTK compiled twice, and no way for a fifth tool to share any of it without an
inter-repo dependency.

The sharing paid for itself immediately: `timers-gui` had been leaking a process
on every dismissal because it carried `toggle_on_relaunch` but not
`leave_when_another_instance_answers`. Both live in
`widgets_gtkui::single_instance` now, and no GUI can have one without the other.

## Using it

```nix
# flake.nix
widgets.url = "github:alansartorio/widgets";
widgets.inputs.nixpkgs.follows = "nixpkgs";
widgets.inputs.flake-utils.follows = "flake-utils";
```

```nix
# home-manager
imports = [ inputs.widgets.homeManagerModules.default ];

services.timers.enable = true;
services.kimai = {
  enable = true;
  url = "https://kimai.example.com";
  tokenFile = osConfig.sops.secrets.kimai_token.path;   # the path, never the token
};
programs.calendar-gui.enable = true;
```

```nix
# NixOS, for the one service that is not yours to run
imports = [ inputs.widgets.nixosModules.claude-usage ];
services.claude-usage = {
  enable = true;
  socketGroupMembers = [ "alice" ];
  tokenRefreshCommand = "ssh -o BatchMode=yes workstation cat .claude/.credentials.json";
};
```

Every module defaults its own packages from this flake, so nothing has to name
a derivation. `homeManagerModules.{timers,kimai,calendar}` and
`nixosModules.{claude-usage,timers,kimai,calendar}` exist for a host that wants
one without the others.

## Feriados

`kimai` and `calendar-gui` share one calendar, fetched from
[api.argentinadatos.com](https://api.argentinadatos.com/v1/feriados/2026) and
cached in `$XDG_STATE_HOME/holidays/`. Fetched rather than computed because the
*puentes turísticos* are declared by decree and follow no rule; a ruleset would
be right about fifteen days a year and silently wrong about three.

What the API does not know — the *días no laborables* like Jueves Santo, and
which puentes this employer actually gives — goes in
`~/.config/holidays/overrides.toml`, which wins per date and is re-read on every
poll. Editing it takes effect at the next tick: no rebuild, no restart. That is
why it is a plain file the module *seeds* rather than manages.

```sh
kimai holidays sync                       # fetch this year
kimai holidays list --month 2026-08
kimai holidays set 2026-04-03 --kind work # se trabaja el Viernes Santo
kimai holidays unset 2026-04-03
```

## Hacking

```sh
nix develop
cargo test --workspace
cargo clippy --workspace --all-targets
nix build .#kimai .#calendar-gui
```

`nix build` deliberately has no default: nine binaries and none of them is the
point of the repo.

One thing to preserve when touching `nix/mkbin.nix`: the `gtk` flag on `mkBin`
is what keeps the CLIs and daemons at ~50 MiB while the GUIs carry GTK's ~956
MiB. Running `wrapGAppsHook4` in the workspace build instead wraps *every*
binary, and each wrapper names the GTK store paths outright — so all nine come
out with the same closure, including the daemons that never open a window.
