<div align="center">
  <img src="data/icon.png" width="120" alt="BootDrive icon"/>

  <h1>BootDrive</h1>

  <p><b>Turn your Linux phone into a bootable USB drive.</b></p>

  <p>
    Pick one disk image and expose your phone to a connected computer as a
    read-only USB&nbsp;CD-ROM or USB&nbsp;disk — then eject to return to normal
    USB behaviour. Developed on postmarketOS (Fairphone&nbsp;6).
  </p>

  <img src="data/screencast.gif" width="280" alt="BootDrive demo"/>
</div>

## Screenshots

<div align="center">
  <img src="data/screenshots/mount.png" width="240" alt="Mount view — connected as a bootable drive"/>
  <img src="data/screenshots/choose.png" width="240" alt="Choose an image from the library"/>
  <img src="data/screenshots/images.png" width="240" alt="Manage the image library"/>
</div>

## Install

BootDrive ships two frontends: a sandboxed **GTK4 / libadwaita Flatpak GUI**
(`net.bresilla.BootDrive`) and a static **CLI** (`bootdrive`). Grab them from the
[latest release](https://github.com/bresilla/bootdrive/releases/latest).

### GUI (Flatpak)

Download the bundle for your architecture, then:

```sh
# phones (aarch64)
flatpak install --user ./bootdrive-aarch64.flatpak
# desktops (x86_64)
flatpak install --user ./bootdrive-x86_64.flatpak

flatpak run net.bresilla.BootDrive
```

The GNOME 48 runtime is pulled from Flathub — add the remote first if you don't
have it:

```sh
flatpak remote-add --if-not-exists --user \
  flathub https://flathub.org/repo/flathub.flatpakrepo
```

### CLI

`bootdrive-cli-aarch64-musl.tar.gz` is a fully static binary — it runs on
Alpine / postmarketOS with no toolchain or glibc:

```sh
tar xzf bootdrive-cli-*.tar.gz
./bootdrive status          # expose | eject | status | watch
```

## Requires: usb-signaller

BootDrive is **unprivileged and sandboxed** — it never touches configfs itself.
The low-level USB-gadget work is done by postmarketOS's
[**usb-signaller**](https://codeberg.org/DylanVanAssche/usb-signaller), which
runs as root and owns the `com.meego.usb_moded` D-Bus interface. BootDrive drives
it, using a small patch that adds a mass-storage / CD-ROM mode:

- **fork:** <https://codeberg.org/bresilla/usb-signaller> (branch `mass-storage-mode`)

Without a compatible `usb-signaller` running, the app still launches — it simply
reports that no compatible USB service is available.

## How it works

1. The GUI copies your chosen image into its own data dir (through the
   file-chooser **portal** — no host filesystem access required).
2. It asks `com.meego.usb_moded` to switch into `mass_storage_mode` /
   `cdrom_mode`.
3. `usb-signaller` binds the USB mass-storage gadget to that image, **read-only**.
4. **Eject** tears the gadget down and returns the phone to normal USB behaviour.

```
GUI / CLI  ──system D-Bus (com.meego.usb_moded)──▶  usb-signaller (root)  ──▶  UDC
```

## Workspace layout

| Crate | Purpose |
| --- | --- |
| `crates/bootdrive-common` | Shared exposure mode, state, and `com.meego.usb_moded` constants |
| `crates/bootdrive-cli` | Native CLI frontend (`bootdrive`) |
| `crates/bootdrive-gui` | GTK4 / libadwaita Flatpak frontend |

## Build from source

Needs Rust and GTK4 / libadwaita. With Nix:

```sh
nix develop
cargo build --release -p bootdrive-gui   # GUI
cargo build --release -p bootdrive-cli   # CLI
```

Flatpak bundle:

```sh
flatpak-builder --user --install --force-clean build data/net.bresilla.BootDrive.yml
```

## License

[MIT](LICENSE) © Trim Bresilla
