#!/usr/bin/env bash
# Cross-compile the BootDrive CLI for postmarketOS (static aarch64-musl).
#
# The CLI has no GTK/C dependency, so it cross-compiles cleanly to a static
# aarch64-unknown-linux-musl binary that runs on the phone with no toolchain —
# scp it to /usr/bin/bootdrive. (The GUI ships as a Flatpak; usb-signaller is
# built with tools/build-usb-signaller-aarch64.sh.)
#
# Requires Nix (for rustup + the aarch64 musl cross toolchain). Run from the
# repository root.
set -euo pipefail

TARGET=aarch64-unknown-linux-musl
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

exec nix shell \
    nixpkgs#rustup \
    "nixpkgs#pkgsCross.aarch64-multiplatform-musl.stdenv.cc" \
    --command bash -c '
set -euo pipefail
export RUSTUP_HOME="$PWD/.xbuild/rustup" CARGO_HOME="$PWD/.xbuild/cargo"
export PATH="$CARGO_HOME/bin:$PATH"

rustup toolchain install stable --profile minimal >/dev/null 2>&1 || true
rustup target add '"$TARGET"' >/dev/null 2>&1 || true

export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=aarch64-unknown-linux-musl-cc
export CC_aarch64_unknown_linux_musl=aarch64-unknown-linux-musl-cc

cargo build --release --target '"$TARGET"' -p bootdrive-cli --bin bootdrive

echo
echo "Built: target/'"$TARGET"'/release/bootdrive"
'
