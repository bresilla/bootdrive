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

# Run the CLI, e.g. `just cli status` or `just cli expose x.iso`.
# Talks to usb-signaller's com.meego.usb_moded on the system bus.
cli *args:
    cargo run --package bootdrive-cli --bin bootdrive -- {{args}}

# Run the GUI outside Flatpak (talks to usb-signaller over system D-Bus).
run-gui:
    cargo run --package bootdrive-gui

# Cross-compile the CLI for postmarketOS (static aarch64-musl).
cross-cli:
    ./tools/build-helper-aarch64.sh

# Cross-compile the patched usb-signaller for postmarketOS (static aarch64-musl).
cross-usb-signaller:
    ./tools/build-usb-signaller-aarch64.sh

# Install/update the GUI Flatpak on the phone from the latest CI build (no sudo).
install-phone host="100.68.168.31":
    ./tools/install-phone.sh {{host}}

# Build + install the patched usb-signaller on the phone (prompts for sudo there).
deploy-usb-signaller host="100.68.168.31":
    ./tools/deploy-usb-signaller.sh {{host}}

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
