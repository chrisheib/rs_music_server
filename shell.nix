let
  # 1. Import nixpkgs
  pkgs = import <nixpkgs> {
    overlays = [
      # 2. Pull in the rust-overlay
      (import (
        builtins.fetchTarball {
          url = "https://github.com/oxalica/rust-overlay/archive/master.tar.gz";
        }
      ))
    ];
  };

  # 3. Now rust-bin is available
  rust = pkgs.rust-bin.stable.latest.default;

  buildInputs = with pkgs; [
    autoAddDriverRunpath
    binutils
    libxkbcommon
    pkg-config
    rust
    libGL
    mold
    openssl
    yt-dlp
    mp3gain
    ffmpeg
  ];
in
pkgs.mkShell {
  inherit buildInputs;
  LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath buildInputs;
  RUSTFLAGS = "-C link-args=-Wl,--no-rosegment,-fuse-ld=mold,-rpath,${pkgs.lib.makeLibraryPath buildInputs}";
  shellHook = ''
    echo "Rust $(rustc --version)"
    echo "ffmpeg $(ffmpeg -version | sed -n "s/ffmpeg version \([-0-9.]*\).*/\1/p;")"
    unset TEMP TMP TEMPDIR TMPDIR
    export RUST_BACKTRACE=1
  '';
}
