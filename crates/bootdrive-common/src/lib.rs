//! Shared types for the BootDrive frontends.
//!
//! BootDrive has **no daemon of its own**. The Flatpak GUI and the CLI both
//! drive postmarketOS's `usb-signaller` directly over its
//! `com.meego.usb_moded` system-D-Bus interface. This crate holds the small
//! pieces both frontends share: the exposure mode, a display state, the D-Bus
//! contract constants, and the current-image symlink helpers.

pub mod protocol;
pub mod state;

pub use protocol::{
    current_image_link, point_current_image, DISPLAY_NAME, MODED_INTERFACE, MODED_PATH,
    MODED_SERVICE, MODE_CDROM, MODE_MASS_STORAGE, MODE_NORMAL,
};
pub use state::{DriveState, ImageMode};
