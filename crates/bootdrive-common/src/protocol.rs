//! The `com.meego.usb_moded` contract BootDrive drives.
//!
//! `usb-signaller` owns this interface on the **system** bus and its policy
//! lets any user call `set_mode`, so the sandboxed Flatpak (with
//! `--system-talk-name=com.meego.usb_moded`) and the CLI can both use it with
//! no extra service, group or PolicyKit action.
//!
//! usb-signaller keeps no runtime state: the image it exposes comes from its
//! `[mass_storage] storage_path` config. We set that once, at install, to
//! [`current_image_link`] and then re-point that symlink at whatever the user
//! selects, so choosing an image needs no privileged write.

use std::path::{Path, PathBuf};

/// Well-known bus name usb-signaller owns on the system bus.
pub const MODED_SERVICE: &str = "com.meego.usb_moded";

/// Object path.
pub const MODED_PATH: &str = "/com/meego/usb_moded";

/// Interface name.
pub const MODED_INTERFACE: &str = "com.meego.usb_moded";

/// Expose the configured image as a USB disk.
pub const MODE_MASS_STORAGE: &str = "mass_storage_mode";

/// Expose the configured image as a USB CD-ROM.
pub const MODE_CDROM: &str = "cdrom_mode";

/// The mode we return to on eject (normal USB networking / developer mode).
pub const MODE_NORMAL: &str = "developer_mode";

/// Application display name.
pub const DISPLAY_NAME: &str = "BootDrive";

/// The fixed path usb-signaller's `[mass_storage] storage_path` points at. It
/// sits in BootDrive's own data directory, which usb-signaller (running as
/// root) can already read, so the frontends can re-point it freely.
pub fn current_image_link() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/root"));
    home.join(".var/app/net.bresilla.BootDrive/data/bootdrive/current.img")
}

/// Point [`current_image_link`] at `target`, so the next mass-storage or
/// CD-ROM mode exposes it.
pub fn point_current_image(target: &Path) -> std::io::Result<()> {
    let link = current_image_link();
    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(target, &link)
}
