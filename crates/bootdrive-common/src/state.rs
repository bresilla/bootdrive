//! The small state model the frontends share. Plain enums; the wire format is
//! `usb-signaller`'s mode strings, so nothing here needs (de)serialization.

use std::str::FromStr;

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
    /// Value passed to usb-signaller's `set_config` (`cdrom=1`/`cdrom=0`).
    pub const fn cdrom_flag(self) -> &'static str {
        match self {
            ImageMode::Cdrom => "1",
            ImageMode::Disk => "0",
        }
    }

    /// A human-friendly label.
    pub const fn label(self) -> &'static str {
        match self {
            ImageMode::Cdrom => "USB CD-ROM",
            ImageMode::Disk => "USB disk",
        }
    }

    /// The default mode for a file with the given extension.
    pub fn default_for_extension(ext: &str) -> ImageMode {
        match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
            "img" | "raw" => ImageMode::Disk,
            _ => ImageMode::Cdrom,
        }
    }
}

/// High-level state shown by the frontends, derived from the current USB mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DriveState {
    /// usb-signaller is missing the `mass_storage_mode` (patch not installed),
    /// or the service is unreachable.
    #[default]
    Unavailable,
    /// Ready to expose an image.
    Idle,
    /// A mode switch is in progress.
    Preparing,
    /// An image is currently exposed as mass storage.
    Active,
    /// Returning to normal USB behaviour.
    Ejecting,
    /// The last operation failed.
    Error,
}

impl DriveState {
    /// A user-facing headline.
    pub const fn headline(self) -> &'static str {
        match self {
            DriveState::Unavailable => "Unavailable",
            DriveState::Idle => "Ready",
            DriveState::Preparing => "Preparing…",
            DriveState::Active => "Active",
            DriveState::Ejecting => "Ejecting…",
            DriveState::Error => "Error",
        }
    }
}

impl FromStr for ImageMode {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "cdrom" => Ok(ImageMode::Cdrom),
            "disk" => Ok(ImageMode::Disk),
            _ => Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_defaults() {
        assert_eq!(ImageMode::default_for_extension("iso"), ImageMode::Cdrom);
        assert_eq!(ImageMode::default_for_extension(".ISO"), ImageMode::Cdrom);
        assert_eq!(ImageMode::default_for_extension("img"), ImageMode::Disk);
        assert_eq!(ImageMode::default_for_extension("raw"), ImageMode::Disk);
    }

    #[test]
    fn cdrom_flag() {
        assert_eq!(ImageMode::Cdrom.cdrom_flag(), "1");
        assert_eq!(ImageMode::Disk.cdrom_flag(), "0");
    }
}
