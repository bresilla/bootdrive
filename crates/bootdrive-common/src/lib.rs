//! Shared types for BootDrive.
//!
//! This crate is depended on by both the privileged daemon (`bootdrived`) and
//! the sandboxed GUI (`bootdrive-gui`). It defines the D-Bus contract, the
//! observable state model and the error type, so both sides agree on the wire
//! format without either pulling in the other's heavy dependencies (GTK on one
//! side, `usb-gadget`/configfs on the other).

pub mod error;
pub mod protocol;
pub mod state;

pub use error::{BootDriveError, ErrorCode};
pub use protocol::{DBUS_INTERFACE, DBUS_OBJECT_PATH, DBUS_SERVICE_NAME, DISPLAY_NAME};
pub use state::{DriveState, ImageMode, StateInfo};
