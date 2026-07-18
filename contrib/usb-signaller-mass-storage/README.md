# usb-signaller mass-storage mode (deferred alternative)

This is the **deferred Option A** from `PLAN.md` §1: instead of BootDrive
shipping its own `bootdrived` service, add a `mass_storage` mode to
postmarketOS's [`usb-signaller`](https://codeberg.org/DylanVanAssche/usb-signaller)
— which already runs as root and owns the `com.meego.usb_moded` D-Bus interface
with a policy that lets any user call `set_mode`/`set_config`.

With this patch, a Flatpak app needs **nothing installed on the host but
itself**: it calls `set_config("image=/path,cdrom=1")` then
`set_mode("mass_storage_mode")`, and ejects with `set_mode("developer_mode")`.

BootDrive does **not** use this by default (it ships the self-contained
`bootdrived` backend instead), because this path depends on the change landing
upstream. The patch is kept here in case it is worth submitting as a merge
request.

## What the patch does

Against usb-signaller `0.3.1`:

- `src/udc.rs`: adds `UDCMode::MassStorage`; `set_mode` creates a
  `mass_storage.0` function and writes `lun.0/{cdrom,ro,removable,file}` (always
  read-only), with full rollback on failure; `load()` detects it.
- `src/main.rs`: maps `"mass_storage_mode"`, advertises it in `get_modes`, adds
  `set_config`/`get_config` D-Bus methods that store the backing image on the
  service object, and threads the image + cdrom flag into `set_mode`.

## Applying

```sh
git clone https://codeberg.org/DylanVanAssche/usb-signaller
cd usb-signaller
git apply /path/to/mass-storage-mode.patch
cargo build --release
```

It compiles cleanly (verified on x86_64 with libdbus). For the phone, build on
device (`apk add cargo dbus-dev`) or cross-compile with a static aarch64 libdbus.
