{
  description = "The bar's tools: claude-usage, timers, kimai and a calendar, over shared plumbing";

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

        # Only the GUIs need system libraries. rusqlite is built with
        # `bundled`, notify-rust talks D-Bus through zbus and ureq brings
        # rustls — all pure Rust — so there is no libsqlite3, no libdbus and no
        # openssl in here, and none is missing.
        nativeBuildInputs = with pkgs; [ pkg-config ];
        buildInputs = with pkgs; [
          glib
          gtk4
          gtk4-layer-shell
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

          # No default. This flake ships nine binaries and no one of them is
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
    );
}
