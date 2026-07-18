//! The shared error type and the stable machine-readable error codes carried by
//! the `ErrorOccurred` signal and D-Bus method errors.

use std::fmt;

/// Stable, machine-readable error codes.
///
/// The GUI matches on these to render a specific recovery hint, so the strings
/// are part of the contract and must stay stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// The mass-storage kernel module could not be loaded.
    MassStorageUnavailable,
    /// No usable USB Device Controller was found.
    NoUdc,
    /// The UDC is currently claimed by something else.
    UdcBusy,
    /// `usb-signaller` could not be stopped / could not release the UDC.
    SignallerReleaseFailed,
    /// The selected file failed validation (missing, not a regular file, empty,
    /// unreadable by the caller, …).
    InvalidImage,
    /// The D-Bus caller is not authorized to activate.
    NotAuthorized,
    /// An operation was requested that is not valid in the current state.
    InvalidState,
    /// A low-level gadget / configfs operation failed.
    GadgetFailure,
    /// Anything else.
    Internal,
}

impl ErrorCode {
    /// The stable wire representation.
    pub const fn as_wire(self) -> &'static str {
        match self {
            ErrorCode::MassStorageUnavailable => "mass-storage-unavailable",
            ErrorCode::NoUdc => "no-udc",
            ErrorCode::UdcBusy => "udc-busy",
            ErrorCode::SignallerReleaseFailed => "signaller-release-failed",
            ErrorCode::InvalidImage => "invalid-image",
            ErrorCode::NotAuthorized => "not-authorized",
            ErrorCode::InvalidState => "invalid-state",
            ErrorCode::GadgetFailure => "gadget-failure",
            ErrorCode::Internal => "internal",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_wire())
    }
}

/// The error type shared across the daemon.
#[derive(Debug, thiserror::Error)]
#[error("{code}: {message}")]
pub struct BootDriveError {
    /// Stable machine-readable code.
    pub code: ErrorCode,
    /// Human-readable, GUI-safe message (never contains a private path).
    pub message: String,
}

impl BootDriveError {
    /// Construct a new error.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        BootDriveError {
            code,
            message: message.into(),
        }
    }

    /// Shorthand for [`ErrorCode::InvalidImage`].
    pub fn invalid_image(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidImage, message)
    }

    /// Shorthand for [`ErrorCode::InvalidState`].
    pub fn invalid_state(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidState, message)
    }

    /// Shorthand for [`ErrorCode::GadgetFailure`].
    pub fn gadget(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::GadgetFailure, message)
    }

    /// Shorthand for [`ErrorCode::Internal`].
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }
}

/// Convenience result alias.
pub type Result<T> = std::result::Result<T, BootDriveError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_code_wire_is_stable() {
        assert_eq!(ErrorCode::NoUdc.as_wire(), "no-udc");
        assert_eq!(ErrorCode::UdcBusy.to_string(), "udc-busy");
    }

    #[test]
    fn error_display_includes_code_and_message() {
        let e = BootDriveError::invalid_image("empty file");
        assert_eq!(e.to_string(), "invalid-image: empty file");
    }
}
