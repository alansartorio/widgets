{
  description = "The bar's tools: claude-usage, timers, kimai, a calendar and a light, over shared plumbing";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      crane,
      flake-utils,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        lib = nixpkgs.lib;
        craneLib = crane.mkLib pkgs;

        # Everything with a C library in nixpkgs links that one, and nothing here
        # compiles a copy of its own. `widgets-http` takes libcurl instead of an
        # HTTP stack in Rust, rusqlite takes libsqlite3 instead of building the
        # amalgamation, and notify-rust takes libdbus instead of zbus.
        #
        # pkg-config is what all three are found with, and it is load-bearing
        # rather than tidy: without it in nativeBuildInputs, curl-sys and
        # libsqlite3-sys do not fail — they quietly fall back to compiling their
        # vendored C sources, which is the thing this arrangement exists to avoid.
        # openssl comes with the `curl` crate's default `ssl` feature: bindings
        # and cert probing, no TLS implementation compiled here.
        nativeBuildInputs = with pkgs; [ pkg-config ];
        buildInputs = with pkgs; [
          curl
          dbus
          glib
          gtk4
          gtk4-layer-shell
          openssl
          sqlite
          wayland
          wayland-protocols
        ];

        inherit
          (import ./nix/mkbin.nix {
            inherit
              lib
              pkgs
              craneLib
              nativeBuildInputs
              buildInputs
              ;
            src = craneLib.path ./.;
          })
          workspace
          mkBin
          ;
      in
      {
        packages = {
          claude-usage = mkBin { name = "claude-usage"; };
          claude-usage-daemon = mkBin { name = "claude-usage-daemon"; };
          claude-usage-gui = mkBin {
            name = "claude-usage-gui";
            gtk = true;
          };

          timers = mkBin { name = "timers"; };
          timers-daemon = mkBin { name = "timers-daemon"; };
          timers-gui = mkBin {
            name = "timers-gui";
            gtk = true;
          };

          # No daemon: kimai keeps nothing between polls that Kimai is not
          # already the authority on.
          kimai = mkBin { name = "kimai"; };
          kimai-gui = mkBin {
            name = "kimai-gui";
            gtk = true;
          };

          calendar-gui = mkBin {
            name = "calendar-gui";
            gtk = true;
          };

          # No CLI and no GUI: the pill is the whole interface, and the bar
          # reaches it through the socket rather than through a binary of ours.
          hass-daemon = mkBin { name = "hass-daemon"; };

          # No default. This flake ships ten binaries and no one of them is
          # the point of it; `nix build` with no attribute should say so rather
          # than pick one at random.
        };

        checks = {
          inherit workspace;
          # The unit tests the workspace build skips, as their own derivation:
          # `nix flake check` runs them, a rebuild does not.
          tests = craneLib.cargoTest {
            pname = "widgets-tests";
            src = craneLib.path ./.;
            inherit nativeBuildInputs buildInputs;
            cargoArtifacts = workspace;
          };
        };

        devShells.default = craneLib.devShell {
          packages = builtins.concatLists [
            (with pkgs; [
              cargo
              rustc
              rustfmt
              clippy
              rust-analyzer
              sqlite
              wrapGAppsHook4
            ])
            nativeBuildInputs
            buildInputs
          ];
        };
      }
    )
    // {
      # Per service *and* an aggregate, because the two are wanted in different
      # places: a host takes `default` and flips three `enable`s, while a module
      # that only wants timers can say so without declaring options for two
      # things it will never turn on.
      #
      # Each closes over `self` so the modules can default their own packages —
      # which is the practical payoff of one flake over four. Nothing importing
      # these has to know what a package is called, or hand the same three
      # derivations across a repo boundary.
      homeManagerModules = {
        timers = import ./nix/modules/timers.nix {
          flavour = "hm";
          inherit self;
        };
        kimai = import ./nix/modules/kimai.nix {
          flavour = "hm";
          inherit self;
        };
        calendar = import ./nix/modules/calendar.nix {
          flavour = "hm";
          inherit self;
        };
        hass = import ./nix/modules/hass.nix {
          flavour = "hm";
          inherit self;
        };
        default = import ./nix/hm.nix { inherit self; };
      };

      nixosModules = {
        # No home-manager flavour: the daemon runs as its own system user, which
        # is the point of it.
        claude-usage = import ./nix/modules/claude-usage.nix {
          flavour = "nixos";
          inherit self;
        };
        timers = import ./nix/modules/timers.nix {
          flavour = "nixos";
          inherit self;
        };
        kimai = import ./nix/modules/kimai.nix {
          flavour = "nixos";
          inherit self;
        };
        calendar = import ./nix/modules/calendar.nix {
          flavour = "nixos";
          inherit self;
        };
        hass = import ./nix/modules/hass.nix {
          flavour = "nixos";
          inherit self;
        };
        default = import ./nix/nixos.nix { inherit self; };
      };
    };
}
