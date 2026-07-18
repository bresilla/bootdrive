//! The USB backend abstraction.
//!
//! Per the plan (section 18) the D-Bus service must not be coupled directly to
//! `usb-gadget`. Everything that touches configfs lives behind [`UsbBackend`],
//! so the whole activation state machine can be exercised on a workstation with
//! no UDC using [`MockUsbBackend`].

use bootdrive_common::{BootDriveError, ImageMode};

use crate::image::ValidatedImage;

/// The configfs name of the gadget BootDrive owns. Only a gadget with *exactly*
/// this name is ever removed by BootDrive.
pub const GADGET_NAME: &str = "bootdrive";

/// What the backend currently believes about the hardware.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BackendState {
    /// Whether an image is currently bound to the UDC.
    pub active: bool,
    /// Name of the UDC in use, if any.
    pub udc: Option<String>,
}

/// Static capabilities discovered by probing the system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendCapabilities {
    /// Name of the UDC that will be used (e.g. `a600000.usb`).
    pub udc: String,
    /// Whether mass-storage support is available.
    pub mass_storage_ok: bool,
}

/// The privileged operations the state machine drives. Implementations own all
/// configfs/UDC interaction.
pub trait UsbBackend: Send {
    /// Confirm a usable UDC and mass-storage support exist.
    fn probe(&mut self) -> Result<BackendCapabilities, BootDriveError>;

    /// Remove a leftover gadget named exactly [`GADGET_NAME`], if present.
    /// Never touches any other gadget directory.
    fn remove_stale_gadget(&mut self) -> Result<(), BootDriveError>;

    /// Unbind (but never remove) any *other* gadget currently bound to the UDC,
    /// so the UDC is free for BootDrive. Used to release `g1` after stopping
    /// `usb-signaller`.
    fn release_foreign_gadgets(&mut self) -> Result<(), BootDriveError>;

    /// Create the `bootdrive` gadget for `image` in `mode` and bind it.
    fn activate(&mut self, image: &ValidatedImage, mode: ImageMode) -> Result<(), BootDriveError>;

    /// Force-eject, unbind and remove the `bootdrive` gadget.
    fn deactivate(&mut self) -> Result<(), BootDriveError>;

    /// Current backend state.
    fn state(&self) -> BackendState;
}

// ---------------------------------------------------------------------------
// Mock backend (used by unit/integration tests and by non-Linux builds)
// ---------------------------------------------------------------------------

/// Scripted behaviour for [`MockUsbBackend`], so tests can drive rollback and
/// recovery paths deterministically.
#[derive(Debug, Clone, Default)]
pub struct MockScript {
    /// `probe` returns these capabilities…
    pub udc: Option<String>,
    /// …with this mass-storage flag.
    pub mass_storage_ok: bool,
    /// If set, `activate` fails with this message (to test rollback).
    pub fail_activate: Option<String>,
    /// If set, `deactivate` fails with this message.
    pub fail_deactivate: Option<String>,
    /// If `true`, a stale gadget is present at startup.
    pub stale_present: bool,
}

/// In-memory backend for tests and non-UDC hosts.
#[derive(Debug, Default)]
pub struct MockUsbBackend {
    script: MockScript,
    active: bool,
    /// Call log for assertions.
    pub calls: Vec<String>,
    /// Set to `true` once a stale gadget has been cleaned.
    pub stale_removed: bool,
    /// Set to `true` once foreign gadgets were released.
    pub foreign_released: bool,
}

impl MockUsbBackend {
    /// Create a mock that reports a working UDC and mass-storage support.
    pub fn working() -> Self {
        MockUsbBackend {
            script: MockScript {
                udc: Some("mock.udc".to_string()),
                mass_storage_ok: true,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Create a mock from an explicit script.
    pub fn with_script(script: MockScript) -> Self {
        MockUsbBackend {
            active: false,
            calls: Vec::new(),
            stale_removed: false,
            foreign_released: false,
            script,
        }
    }
}

impl UsbBackend for MockUsbBackend {
    fn probe(&mut self) -> Result<BackendCapabilities, BootDriveError> {
        self.calls.push("probe".into());
        match (&self.script.udc, self.script.mass_storage_ok) {
            (Some(udc), true) => Ok(BackendCapabilities {
                udc: udc.clone(),
                mass_storage_ok: true,
            }),
            (Some(_), false) => Err(BootDriveError::new(
                bootdrive_common::ErrorCode::MassStorageUnavailable,
                "mass-storage support is unavailable",
            )),
            (None, _) => Err(BootDriveError::new(
                bootdrive_common::ErrorCode::NoUdc,
                "no usable USB device controller was found",
            )),
        }
    }

    fn remove_stale_gadget(&mut self) -> Result<(), BootDriveError> {
        self.calls.push("remove_stale_gadget".into());
        if self.script.stale_present {
            self.stale_removed = true;
            self.script.stale_present = false;
        }
        Ok(())
    }

    fn release_foreign_gadgets(&mut self) -> Result<(), BootDriveError> {
        self.calls.push("release_foreign_gadgets".into());
        self.foreign_released = true;
        Ok(())
    }

    fn activate(
        &mut self,
        _image: &ValidatedImage,
        _mode: ImageMode,
    ) -> Result<(), BootDriveError> {
        self.calls.push("activate".into());
        if let Some(msg) = &self.script.fail_activate {
            return Err(BootDriveError::gadget(msg.clone()));
        }
        self.active = true;
        Ok(())
    }

    fn deactivate(&mut self) -> Result<(), BootDriveError> {
        self.calls.push("deactivate".into());
        if let Some(msg) = &self.script.fail_deactivate {
            return Err(BootDriveError::gadget(msg.clone()));
        }
        self.active = false;
        Ok(())
    }

    fn state(&self) -> BackendState {
        BackendState {
            active: self.active,
            udc: self.script.udc.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Real backend (Linux + usb-gadget)
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
pub use real::UsbGadgetBackend;

#[cfg(target_os = "linux")]
mod real {
    use super::*;
    use bootdrive_common::ErrorCode;
    use usb_gadget::{
        default_udc,
        function::msd::{Lun, Msd},
        registered, Class, Config, Gadget, Id, RegGadget, Strings, Udc,
    };

    /// The live objects that must be kept alive while an image is exposed.
    /// Dropping `RegGadget` would remove the gadget, so it must never be a
    /// temporary local (plan section 11).
    struct ActiveGadget {
        _udc: Udc,
        gadget: RegGadget,
        _msd: Msd,
    }

    /// Production backend backed by configfs via `usb-gadget`.
    #[derive(Default)]
    pub struct UsbGadgetBackend {
        active: Option<ActiveGadget>,
        udc_name: Option<String>,
    }

    impl UsbGadgetBackend {
        /// Create a new, inactive backend.
        pub fn new() -> Self {
            Self::default()
        }

        fn find_udc() -> Result<Udc, BootDriveError> {
            default_udc().map_err(|e| {
                BootDriveError::new(
                    ErrorCode::NoUdc,
                    format!("no usable USB device controller: {e}"),
                )
            })
        }
    }

    impl UsbBackend for UsbGadgetBackend {
        fn probe(&mut self) -> Result<BackendCapabilities, BootDriveError> {
            let udc = Self::find_udc()?;
            let name = udc.name().to_string_lossy().into_owned();
            // Attempting to load libcomposite/usb_f_mass_storage happens lazily
            // inside usb-gadget; we treat presence of a UDC plus a readable
            // configfs as the probe signal here and let activate() surface a
            // precise mass-storage error if the module is genuinely missing.
            self.udc_name = Some(name.clone());
            Ok(BackendCapabilities {
                udc: name,
                mass_storage_ok: true,
            })
        }

        fn remove_stale_gadget(&mut self) -> Result<(), BootDriveError> {
            let gadgets = registered()
                .map_err(|e| BootDriveError::gadget(format!("could not enumerate gadgets: {e}")))?;
            for g in gadgets {
                if g.name() == std::ffi::OsStr::new(GADGET_NAME) {
                    tracing::warn!("removing stale '{GADGET_NAME}' gadget left by a previous run");
                    g.remove().map_err(|e| {
                        BootDriveError::gadget(format!("could not remove stale gadget: {e}"))
                    })?;
                }
                // Any other gadget is deliberately left untouched.
            }
            Ok(())
        }

        fn release_foreign_gadgets(&mut self) -> Result<(), BootDriveError> {
            let gadgets = registered()
                .map_err(|e| BootDriveError::gadget(format!("could not enumerate gadgets: {e}")))?;
            for g in gadgets {
                if g.name() == std::ffi::OsStr::new(GADGET_NAME) {
                    continue;
                }
                match g.udc() {
                    Ok(Some(_)) => {
                        // Bound to a UDC but not ours: unbind without removing.
                        // Gadgets from registered() have attached=false, so this
                        // never deletes them (e.g. postmarketOS's `g1`).
                        tracing::info!(
                            "temporarily unbinding foreign gadget {:?} to free the UDC",
                            g.name()
                        );
                        g.bind(None).map_err(|e| {
                            BootDriveError::new(
                                ErrorCode::UdcBusy,
                                format!("could not release the UDC from another gadget: {e}"),
                            )
                        })?;
                    }
                    _ => continue,
                }
            }
            Ok(())
        }

        fn activate(
            &mut self,
            image: &ValidatedImage,
            mode: ImageMode,
        ) -> Result<(), BootDriveError> {
            if self.active.is_some() {
                return Err(BootDriveError::invalid_state("already active"));
            }

            let mut lun = Lun::new(&image.path).map_err(|e| {
                BootDriveError::new(
                    ErrorCode::MassStorageUnavailable,
                    format!("mass-storage backing file could not be opened: {e}"),
                )
            })?;
            lun.read_only = true;
            lun.cdrom = mode == ImageMode::Cdrom;
            lun.removable = true;
            lun.inquiry_string = "BootDrive".into();

            let (msd, handle) = Msd::builder().with_lun(lun).build();

            let config = Config::new("Bootable image").with_function(handle);

            // A short, stable serial derived from the display name keeps hosts
            // from caching a mismatched descriptor across images.
            let serial = format!("bootdrive-{}", image.size);

            let mut gadget = Gadget::new(
                Class::INTERFACE_SPECIFIC,
                Id::LINUX_FOUNDATION_STORAGE,
                Strings::new("BootDrive", "Bootable Image", serial),
            )
            .with_config(config);
            gadget.name = Some(GADGET_NAME.to_string());

            let udc = Self::find_udc()?;
            let udc_name = udc.name().to_string_lossy().into_owned();

            let reg = gadget.bind(&udc).map_err(|e| {
                BootDriveError::new(
                    ErrorCode::GadgetFailure,
                    format!("could not bind the USB gadget: {e}"),
                )
            })?;

            self.udc_name = Some(udc_name);
            self.active = Some(ActiveGadget {
                _udc: udc,
                gadget: reg,
                _msd: msd,
            });
            Ok(())
        }

        fn deactivate(&mut self) -> Result<(), BootDriveError> {
            let Some(active) = self.active.take() else {
                return Ok(());
            };
            // Unbind from the UDC first for a clean host disconnect, then remove.
            let _ = active.gadget.bind(None);
            active
                .gadget
                .remove()
                .map_err(|e| BootDriveError::gadget(format!("could not remove the gadget: {e}")))?;
            Ok(())
        }

        fn state(&self) -> BackendState {
            BackendState {
                active: self.active.is_some(),
                udc: self.udc_name.clone(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn image() -> ValidatedImage {
        ValidatedImage {
            path: PathBuf::from("/tmp/x.iso"),
            display_name: "x.iso".into(),
            size: 1024,
        }
    }

    #[test]
    fn mock_activate_deactivate_cycle() {
        let mut b = MockUsbBackend::working();
        assert!(!b.state().active);
        b.activate(&image(), ImageMode::Cdrom).unwrap();
        assert!(b.state().active);
        b.deactivate().unwrap();
        assert!(!b.state().active);
    }

    #[test]
    fn mock_probe_reports_no_udc() {
        let mut b = MockUsbBackend::with_script(MockScript::default());
        let err = b.probe().unwrap_err();
        assert_eq!(err.code, bootdrive_common::ErrorCode::NoUdc);
    }

    #[test]
    fn mock_activate_failure_is_surfaced() {
        let mut b = MockUsbBackend::with_script(MockScript {
            udc: Some("u".into()),
            mass_storage_ok: true,
            fail_activate: Some("boom".into()),
            ..Default::default()
        });
        assert!(b.activate(&image(), ImageMode::Cdrom).is_err());
        assert!(!b.state().active);
    }
}
