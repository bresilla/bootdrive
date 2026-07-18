//! The D-Bus contract constants shared by the backend and both frontends.
//!
//! `bootdrived` serves this interface on the **system** bus; the CLI and the
//! Flatpak GUI proxy it. Importing these here keeps the bus name, object path
//! and interface name from ever drifting out of sync.

/// Well-known bus name owned by the backend on the **system** bus.
pub const DBUS_SERVICE_NAME: &str = "net.bresilla.BootDrive1";

/// Object path the interface is exported on.
pub const DBUS_OBJECT_PATH: &str = "/net/bresilla/BootDrive1";

/// Interface name.
pub const DBUS_INTERFACE: &str = "net.bresilla.BootDrive1";

/// Application display name.
pub const DISPLAY_NAME: &str = "BootDrive";
