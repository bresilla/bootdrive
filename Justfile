# BootDrive development commands.
# Run `just` inside `nix develop` so GTK4/libadwaita/D-Bus are available.

# List available recipes.
default:
    @just --list

# Full local gate: format, lint, test.
check:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo test --workspace

# Auto-format the workspace.
fmt:
    cargo fmt --all

# Run the backend service (needs root for real USB gadget access).
run-daemon:
    sudo -E cargo run --package bootdrived --bin bootdrived

# Run the CLI frontend, e.g. `just cli status` or `just cli expose x.iso`.
cli *args:
    cargo run --package bootdrive-cli --bin bootdrive -- {{args}}

# Run the GUI outside Flatpak (talks to the running backend over system D-Bus).
run-gui:
    cargo run --package bootdrive-gui

# Cross-compile the backend + CLI for postmarketOS (static aarch64-musl).
cross-backend:
    ./tools/build-helper-aarch64.sh

# Build, copy, and install the backend on the phone (prompts for sudo there).
# Usage: just deploy            (default host)
#        just deploy 100.x.x.x  (explicit host)
deploy host="100.68.168.31":
    ./tools/deploy-phone.sh {{host}}

# Install/update the GUI Flatpak on the phone from the latest CI build (no sudo).
install-phone host="100.68.168.31":
    ./tools/install-phone.sh {{host}}

# Low-level hardware proof: expose one ISO, then Ctrl-C to clean up.
probe image:
    sudo -E cargo run --package bootdrived --bin probe -- {{image}}

# Regenerate the offline Cargo sources for the Flatpak build.
flatpak-sources:
    ./tools/update-flatpak-sources.sh

# Build and install the Flatpak locally.
flatpak-build:
    flatpak-builder --force-clean --user --install-deps-from=flathub --install \
        build-flatpak data/net.bresilla.BootDrive.yml

# Run the installed Flatpak.
flatpak-run:
    flatpak run net.bresilla.BootDrive

# Validate desktop + AppStream metadata.
validate-data:
    desktop-file-validate data/net.bresilla.BootDrive.desktop
    appstreamcli validate --no-net data/net.bresilla.BootDrive.metainfo.xml
