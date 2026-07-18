//! `bootdrived` — the privileged BootDrive helper.
//!
//! The crate is split into small, individually testable modules and exposed as
//! a library so both the `bootdrived` service binary and the `probe`
//! diagnostic binary can share the same backend code.
//!
//! Module map:
//! - [`image`] — path/permission validation of the selected file.
//! - [`usb`] — the [`usb::UsbBackend`] abstraction plus real/mock backends.
//! - [`usb_signaller`] — handoff with postmarketOS's `usb-signaller`.
//! - [`recovery`] — crash-safe runtime state under `/run/bootdrived`.
//! - [`state`] — the transactional activation state machine.
//! - [`service`] — the system-D-Bus adapter.
//! - [`authorization`] — caller access control.
//! - [`caller`] — resolving the caller's uid/gids for validation.

pub mod authorization;
pub mod caller;
pub mod image;
pub mod recovery;
pub mod service;
pub mod state;
pub mod usb;
pub mod usb_signaller;
