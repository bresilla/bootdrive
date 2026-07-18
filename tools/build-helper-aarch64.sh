#!/usr/bin/env bash
# Cross-compile the BootDrive backend + CLI for postmarketOS (aarch64), static.
#
# Neither binary depends on GTK, so both cross-compile cleanly to
# aarch64-unknown-linux-musl from an x86_64 workstation. The results are static
# binaries that run on the phone with no toolchain or glibc installed — scp
# bootdrived to /usr/libexec/bootdrived and bootdrive to /usr/bin/bootdrive.
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

cargo build --release --target '"$TARGET"' \
    -p bootdrived --bin bootdrived --bin probe \
    -p bootdrive-cli --bin bootdrive

echo
echo "Built:"
echo "  target/'"$TARGET"'/release/bootdrived"
echo "  target/'"$TARGET"'/release/bootdrive"
echo "  target/'"$TARGET"'/release/probe"
'
