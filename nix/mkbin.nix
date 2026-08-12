# One build of the whole workspace, and one thin derivation per binary.
#
# Building each package separately would compile serde, chrono and relm4 once
# per crate; building them together and then splitting is what keeps that to
# one pass. The split itself is not cosmetic either — Nix derives a package's
# closure by scanning its files for store paths, so a bare binary that never
# links GTK does not reference it, even though the workspace it was built in
# does.
#
# `gtk` is what makes that split hold. Running wrapGAppsHook4 in the workspace
# build wraps *every* binary in its bin/, and each wrapper names the GTK store
# paths outright: all packages then come out with an identical ~960 MB closure,
# GTK and all, including the daemons that never open a window and the CLIs the
# bar spawns sixty times an hour. Wrapping here instead means only the GUIs pay
# for it — and with nine binaries in this workspace rather than three, that is
# now the difference between one bloated closure and six.
{
  lib,
  pkgs,
  craneLib,
  src,
  nativeBuildInputs,
  buildInputs,
}:
let
  workspace = craneLib.buildPackage {
    # Both a `pname` here and on each binary below: a virtual workspace root
    # has no package name for crane to infer, and without one the store paths
    # come out anonymous.
    pname = "widgets-workspace";
    inherit
      src
      nativeBuildInputs
      buildInputs
      ;
    cargoExtraArgs = "--workspace";
    # The tests are pure unit tests over parsing, formatting and holiday
    # arithmetic; they run in the devshell and in `nix flake check`, where a
    # failure is something to fix rather than something that blocks every
    # machine's rebuild.
    doCheck = false;
  };

  mkBin =
    {
      name,
      gtk ? false,
    }:
    pkgs.stdenv.mkDerivation {
      pname = name;
      inherit (workspace) version;
      src = workspace;

      nativeBuildInputs = lib.optionals gtk [ pkgs.wrapGAppsHook4 ];
      buildInputs = lib.optionals gtk buildInputs;

      installPhase = ''
        runHook preInstall
        mkdir -p $out/bin
        install -m 0755 $src/bin/${name} $out/bin/${name}
        runHook postInstall
      '';
    };
in
{
  inherit workspace mkBin;
}
