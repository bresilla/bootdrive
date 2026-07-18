# BootDrive

Turn a postmarketOS phone (target: Fairphone 6) into a bootable USB drive.
Select one ISO or raw disk image and expose the phone to a connected computer
as a bootable USB CD-ROM or USB disk.

## Architecture

A Flatpak app cannot be root (verified — the sandbox blocks configfs writes), so
the privileged USB-gadget work is done by postmarketOS's **`usb-signaller`**,
which already runs as root and owns the `com.meego.usb_moded` D-Bus interface.
BootDrive adds a **`mass_storage_mode`** to usb-signaller (see
[`contrib/usb-signaller-mass-storage/`](contrib/usb-signaller-mass-storage/) and
the fork at <https://codeberg.org/bresilla/usb-signaller>), then ships only two
thin frontends that drive it:

- **`bootdrive`** — a native CLI (`expose`/`eject`/`status`/`watch`).
- **`net.bresilla.BootDrive`** — a sandboxed Flatpak GTK4/libadwaita GUI.

To expose an image a frontend calls `set_config("image=…,cdrom=…")` then
`set_mode("mass_storage_mode")`; eject is `set_mode("developer_mode")`.
usb-signaller's policy already lets any user call these, so there is **no
BootDrive daemon, group or PolicyKit action** — install the Flatpak (plus a
usb-signaller with the patch) and go.

```
CLI / GUI (Flatpak)  ──system D-Bus (com.meego.usb_moded)──▶  usb-signaller (root)  ──▶  UDC
```

See [PLAN.md](PLAN.md) for the full design and [FLATHUB.md](FLATHUB.md) for CI /
Flathub.

## Workspace layout

| Crate | Purpose |
| --- | --- |
| `crates/bootdrive-common` | Shared exposure mode, state, and `com.meego.usb_moded` constants |
| `crates/bootdrive-cli` | Native CLI frontend (`bootdrive`) |
| `crates/bootdrive-gui` | GTK4/libadwaita Flatpak frontend |

The `mass_storage_mode` patch to usb-signaller lives in
`contrib/usb-signaller-mass-storage/`.

## Development

The Nix shell provides the full toolchain (Rust, GTK4, libadwaita, D-Bus,
PolicyKit, Flatpak, AppStream/desktop validators, `just`):

```sh
nix develop
just check          # fmt + clippy + tests
just cli status     # drive usb-signaller from the CLI
just run-gui        # run the GUI outside Flatpak
```

The CLI and GUI talk to `com.meego.usb_moded` on the system bus, so they need a
running (patched) usb-signaller to do anything.

For aarch64: `just cross-cli` builds a static `bootdrive` CLI, and
`just cross-usb-signaller` builds the patched usb-signaller — both copyable
straight onto the phone.

## The privileged side: patched usb-signaller

The `mass_storage_mode` is a change to usb-signaller itself (Rust, on Codeberg):

- Patch + notes: `contrib/usb-signaller-mass-storage/`.
- Fork with the patch applied: <https://codeberg.org/bresilla/usb-signaller>
  (branch `mass-storage-mode`), proposed upstream to
  <https://codeberg.org/DylanVanAssche/usb-signaller>.

Until it lands upstream, run the patched build on the phone (replace
`/usr/bin/usb-signaller` and restart the service).

## Packaging

- **Flatpak (GUI):** built for x86_64 + aarch64 by CI (`.github/workflows/flatpak.yml`)
  — see [FLATHUB.md](FLATHUB.md). Local: `just flatpak-sources` then
  `just flatpak-build`. Fully offline via `data/cargo-sources.json`.
- **CLI:** static aarch64 binary attached to GitHub releases, or `just cross-cli`.

## Status

The CLI, GUI, and the usb-signaller `mass_storage_mode` patch build and are
being validated on a Fairphone 6 running postmarketOS.

## License

MIT
