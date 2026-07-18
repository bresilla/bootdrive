#!/bin/sh
# Regenerate the offline Cargo sources used by the Flatpak build.
#
# Flatpak builds have no general network access, so every crate must be
# vendored ahead of time. This script turns Cargo.lock into
# data/cargo-sources.json using the Cargo generator from flatpak-builder-tools.
#
# Requires: python3 with the `aiohttp` and `toml` modules (provided by the Nix
# dev shell). Run from the repository root.
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

GENERATOR="tools/flatpak-cargo-generator.py"
GENERATOR_URL="https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py"

if [ ! -f Cargo.lock ]; then
    echo "Cargo.lock not found; run 'cargo generate-lockfile' first." >&2
    exit 1
fi

if [ ! -f "$GENERATOR" ]; then
    echo "Fetching flatpak-cargo-generator.py…"
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$GENERATOR_URL" -o "$GENERATOR"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "$GENERATOR" "$GENERATOR_URL"
    else
        echo "Neither curl nor wget is available to download the generator." >&2
        exit 1
    fi
fi

echo "Generating data/cargo-sources.json from Cargo.lock…"
python3 "$GENERATOR" Cargo.lock -o data/cargo-sources.json

echo "Done. Commit data/cargo-sources.json alongside Cargo.lock."
