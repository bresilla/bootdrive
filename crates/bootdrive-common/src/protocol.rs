//! The `com.meego.usb_moded` contract BootDrive drives.
//!
//! `usb-signaller` owns this interface on the **system** bus and its policy
//! already lets any user call `set_mode`/`set_config`, so the sandboxed Flatpak
//! (with `--system-talk-name=com.meego.usb_moded`) and the CLI can both use it
//! with no extra service, group or PolicyKit action.

/// Well-known bus name usb-signaller owns on the system bus.
pub const MODED_SERVICE: &str = "com.meego.usb_moded";

/// Object path.
pub const MODED_PATH: &str = "/com/meego/usb_moded";

/// Interface name.
pub const MODED_INTERFACE: &str = "com.meego.usb_moded";

/// The mode our patch adds: expose the configured image as USB mass storage.
pub const MODE_MASS_STORAGE: &str = "mass_storage_mode";

/// The mode we return to on eject (normal USB networking / developer mode).
pub const MODE_NORMAL: &str = "developer_mode";

/// Application display name.
pub const DISPLAY_NAME: &str = "BootDrive";
