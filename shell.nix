with import <nixpkgs> {
  overlays = [ (import (builtins.fetchTarball https://github.com/mozilla/nixpkgs-mozilla/archive/master.tar.gz)) ];
};

stdenv.mkDerivation {
  name = "marker";
  buildInputs = [
    openssl
    pkgconfig
    (rustChannels.stable.rust.override {
      extensions = [ "clippy-preview" ];
    })
  ];
}
