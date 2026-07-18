#!/usr/bin/env bash
# Cross-compile the patched usb-signaller for postmarketOS (static aarch64-musl).
#
# usb-signaller links libdbus (C), so this needs a static aarch64 libdbus from
# nixpkgs in addition to the Rust cross toolchain. Point SIGNALLER_DIR at your
# usb-signaller checkout (with the mass-storage patch on the current branch).
set -euo pipefail

TARGET=aarch64-unknown-linux-musl
SIGNALLER_DIR="${SIGNALLER_DIR:-/home/bresilla/data/code/github/usb-signaller}"
cd "$SIGNALLER_DIR"

exec nix shell \
  nixpkgs#rustup \
  "nixpkgs#pkgsCross.aarch64-multiplatform-musl.stdenv.cc" \
  "nixpkgs#pkgsCross.aarch64-multiplatform-musl.pkgsStatic.dbus" \
  nixpkgs#pkg-config \
  --command bash -c '
set -euo pipefail
export RUSTUP_HOME="$PWD/.xbuild/rustup" CARGO_HOME="$PWD/.xbuild/cargo"
export PATH="$CARGO_HOME/bin:$PATH"
rustup toolchain install stable --profile minimal >/dev/null 2>&1 || true
rustup target add '"$TARGET"' >/dev/null 2>&1 || true

PC=$(find /nix/store -maxdepth 4 -name dbus-1.pc -path "*aarch64*musl*" 2>/dev/null | grep -i static | head -1)
export PKG_CONFIG_PATH="$(dirname "$PC")"
export PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_ALL_STATIC=1
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=aarch64-unknown-linux-musl-cc
export CC_aarch64_unknown_linux_musl=aarch64-unknown-linux-musl-cc
export RUSTFLAGS="-C target-feature=+crt-static"

cargo build --release --target '"$TARGET"'
echo "Built: target/'"$TARGET"'/release/usb-signaller"
'
