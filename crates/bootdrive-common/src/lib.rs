//! Shared types for the BootDrive frontends.
//!
//! BootDrive has **no daemon of its own**. The Flatpak GUI and the CLI both
//! drive postmarketOS's `usb-signaller` directly over its
//! `com.meego.usb_moded` system-D-Bus interface (with the `mass_storage_mode`
//! added by our patch). This crate holds the small pieces both frontends
//! share: the exposure mode, a display state, and the D-Bus contract constants.

pub mod protocol;
pub mod state;

pub use protocol::{
    DISPLAY_NAME, MODED_INTERFACE, MODED_PATH, MODED_SERVICE, MODE_MASS_STORAGE, MODE_NORMAL,
};
pub use state::{DriveState, ImageMode};
