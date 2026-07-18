# BootDrive

Turn a postmarketOS phone (target: Fairphone 6) into a bootable USB drive.
Select one ISO or raw disk image and expose the phone to a connected computer
as a bootable USB CD-ROM or USB disk.

## Architecture

A Flatpak app cannot be root (verified — the sandbox blocks configfs writes), so
BootDrive is one privileged backend with two thin frontends:

- **`bootdrived`** — a native postmarketOS **service** running as root. It owns
  all the logic: the USB mass-storage gadget via configfs (using
  [`usb-gadget`](https://github.com/surban/usb-gadget)), image validation, the
  UDC handoff with `usb-signaller`, and crash recovery. It exposes the **system**
  D-Bus interface `net.bresilla.BootDrive1`.
- **`bootdrive`** — a native CLI frontend (`expose`/`eject`/`status`/`watch`).
- **`net.bresilla.BootDrive`** — a sandboxed Flatpak GTK4/libadwaita GUI.

Both frontends are thin clients; an exposed image stays up until something
ejects it (closing the GUI does not tear it down).

```
CLI / GUI (Flatpak)  ──system D-Bus──▶  bootdrived (root)  ──configfs──▶  UDC
```

See [PLAN.md](PLAN.md) for the full design.

## Workspace layout

| Crate | Purpose |
| --- | --- |
| `crates/bootdrive-common` | Shared D-Bus contract, state model and errors |
| `crates/bootdrived` | Privileged backend service + `probe` diagnostic binary |
| `crates/bootdrive-cli` | Native CLI frontend (`bootdrive`) |
| `crates/bootdrive-gui` | GTK4/libadwaita Flatpak frontend |

The backend and CLI keep `usb-gadget`/GTK out of each other's dependency trees;
the frontends share only `bootdrive-common`.

## Development

The Nix shell provides the full toolchain (Rust, GTK4, libadwaita, D-Bus,
PolicyKit, Flatpak, AppStream/desktop validators, `just`):

```sh
nix develop
just check          # fmt + clippy + tests
just run-daemon     # run the backend (needs root for real gadget access)
just cli status     # drive it from the CLI
just run-gui        # run the GUI outside Flatpak
```

The backend's state machine, path validation, authorization, rollback and
recovery are all unit-tested against mock backends, so `just check` passes on a
workstation with no USB device controller.

For aarch64: `just cross-backend` builds static `bootdrived`/`bootdrive` binaries
you can copy straight onto the phone.

### Hardware proof

On the target device, validate the whole handoff before anything else:

```sh
sudo -E cargo run --package bootdrived --bin probe -- /path/to/image.iso
```

It releases `usb-signaller`, exposes the ISO read-only as a CD-ROM, waits for
Ctrl-C, then cleans up and restores normal USB behaviour.

## Packaging

- **Flatpak (GUI):** `just flatpak-sources` then `just flatpak-build`. The build
  is fully offline using `data/cargo-sources.json`.
- **postmarketOS (daemon):** see `packaging/alpine/APKBUILD`. Installs the
  daemon, OpenRC service, system D-Bus policy and PolicyKit action.

## Status

Milestones 1–3 (workspace, native daemon, GUI) are implemented and tested on the
workstation. Milestones 0/4/5 (on-device proof, Flatpak build, apk install) and
the Fairphone 6 integration tests require the target hardware.

## License

MIT
