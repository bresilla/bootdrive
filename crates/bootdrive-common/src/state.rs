//! The observable state model shared between the helper and the GUI.
//!
//! The enums are serialized as strings rather than integers. Strings keep the
//! pipe protocol self-describing and forward compatible: an older peer that
//! receives an unknown state string can fall back to [`DriveState::Unavailable`]
//! instead of misinterpreting an integer.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use zvariant::Type;

/// High-level lifecycle state of the USB gadget owned by the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DriveState {
    /// No usable UDC / mass-storage support, or the daemon cannot operate.
    #[default]
    Unavailable,
    /// Ready to expose an image; nothing is currently bound.
    Idle,
    /// A transactional activation is in progress.
    Preparing,
    /// An image is currently exposed to the host over USB.
    Active,
    /// A transactional deactivation is in progress.
    Ejecting,
    /// The last operation failed; see [`StateInfo::last_error`].
    Error,
}

impl DriveState {
    /// The stable wire representation used on D-Bus.
    pub const fn as_wire(self) -> &'static str {
        match self {
            DriveState::Unavailable => "unavailable",
            DriveState::Idle => "idle",
            DriveState::Preparing => "preparing",
            DriveState::Active => "active",
            DriveState::Ejecting => "ejecting",
            DriveState::Error => "error",
        }
    }
}

impl fmt::Display for DriveState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_wire())
    }
}

impl FromStr for DriveState {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "unavailable" => DriveState::Unavailable,
            "idle" => DriveState::Idle,
            "preparing" => DriveState::Preparing,
            "active" => DriveState::Active,
            "ejecting" => DriveState::Ejecting,
            "error" => DriveState::Error,
            _ => return Err(()),
        })
    }
}

/// How the image is presented to the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageMode {
    /// Emulated USB optical drive; the default for `.iso` files.
    #[default]
    Cdrom,
    /// Plain USB disk; the default for `.img` / `.raw` files.
    Disk,
}

impl ImageMode {
    /// The stable wire representation used on D-Bus.
    pub const fn as_wire(self) -> &'static str {
        match self {
            ImageMode::Cdrom => "cdrom",
            ImageMode::Disk => "disk",
        }
    }

    /// A human-friendly label for the GUI.
    pub const fn label(self) -> &'static str {
        match self {
            ImageMode::Cdrom => "USB CD-ROM",
            ImageMode::Disk => "USB disk",
        }
    }

    /// The default mode for a file with the given (lower-cased) extension.
    pub fn default_for_extension(ext: &str) -> ImageMode {
        match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
            "img" | "raw" => ImageMode::Disk,
            _ => ImageMode::Cdrom,
        }
    }
}

impl fmt::Display for ImageMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_wire())
    }
}

impl FromStr for ImageMode {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "cdrom" => ImageMode::Cdrom,
            "disk" => ImageMode::Disk,
            _ => return Err(()),
        })
    }
}

// --- D-Bus (de)serialization as strings -------------------------------------

macro_rules! string_wire_impls {
    ($ty:ty, $fallback:expr) => {
        impl Type for $ty {
            const SIGNATURE: &'static zvariant::Signature = <str as Type>::SIGNATURE;
        }

        impl Serialize for $ty {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_wire())
            }
        }

        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let raw = String::deserialize(deserializer)?;
                Ok(raw.parse().unwrap_or($fallback))
            }
        }
    };
}

string_wire_impls!(DriveState, DriveState::Unavailable);
string_wire_impls!(ImageMode, ImageMode::Cdrom);

/// The full state snapshot the helper reports to the GUI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct StateInfo {
    /// Current lifecycle state.
    pub state: DriveState,
    /// Display name of the selected image (never a full private path).
    pub display_name: String,
    /// The mode the image is (or would be) exposed in.
    pub mode: ImageMode,
    /// Name of the UDC in use, or empty when none is bound.
    pub udc: String,
    /// Whether `usb-signaller` was running before activation and should be
    /// restored on deactivation.
    pub signaller_was_running: bool,
    /// Human-readable text for the most recent error, or empty.
    pub last_error: String,
}

impl Default for StateInfo {
    fn default() -> Self {
        StateInfo {
            state: DriveState::Unavailable,
            display_name: String::new(),
            mode: ImageMode::Cdrom,
            udc: String::new(),
            signaller_was_running: false,
            last_error: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_state_round_trips_through_wire() {
        for s in [
            DriveState::Unavailable,
            DriveState::Idle,
            DriveState::Preparing,
            DriveState::Active,
            DriveState::Ejecting,
            DriveState::Error,
        ] {
            assert_eq!(s.as_wire().parse::<DriveState>().unwrap(), s);
        }
    }

    #[test]
    fn image_mode_round_trips_through_wire() {
        assert_eq!("cdrom".parse::<ImageMode>().unwrap(), ImageMode::Cdrom);
        assert_eq!("disk".parse::<ImageMode>().unwrap(), ImageMode::Disk);
    }

    #[test]
    fn unknown_state_string_falls_back() {
        assert!("bogus".parse::<DriveState>().is_err());
    }

    #[test]
    fn extension_defaults_match_plan() {
        assert_eq!(ImageMode::default_for_extension("iso"), ImageMode::Cdrom);
        assert_eq!(ImageMode::default_for_extension(".ISO"), ImageMode::Cdrom);
        assert_eq!(ImageMode::default_for_extension("img"), ImageMode::Disk);
        assert_eq!(ImageMode::default_for_extension("raw"), ImageMode::Disk);
    }
}
