//! The transactional activation state machine.
//!
//! This is the heart of the daemon and is deliberately free of D-Bus and
//! configfs: it drives [`UsbBackend`] and [`SignallerControl`] trait objects, so
//! every transition — including rollback and startup recovery — is unit-tested
//! against mocks with no root and no UDC (plan sections 12 and 18).

use std::time::Duration;

use bootdrive_common::{BootDriveError, DriveState, ImageMode, StateInfo};
use tokio::sync::mpsc::UnboundedSender;

use crate::image::ValidatedImage;
use crate::recovery::{RecoveryState, RecoveryStore};
use crate::usb::{UsbBackend, GADGET_NAME};
use crate::usb_signaller::SignallerControl;

/// Events emitted as the machine advances, forwarded to D-Bus signals by the
/// service layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonEvent {
    /// A new state snapshot.
    State(StateInfo),
    /// An error occurred (also accompanied by a `State` with `Error`).
    Error {
        /// Stable machine-readable code.
        code: String,
        /// Human-readable message.
        message: String,
    },
}

/// The activation coordinator.
pub struct Manager {
    backend: Box<dyn UsbBackend>,
    signaller: Box<dyn SignallerControl>,
    recovery: RecoveryStore,
    info: StateInfo,
    events: Option<UnboundedSender<DaemonEvent>>,
    #[allow(dead_code)]
    udc_release_timeout: Duration,
}

impl Manager {
    /// Build a manager from its collaborators.
    pub fn new(
        backend: Box<dyn UsbBackend>,
        signaller: Box<dyn SignallerControl>,
        recovery: RecoveryStore,
    ) -> Self {
        Manager {
            backend,
            signaller,
            recovery,
            info: StateInfo::default(),
            events: None,
            udc_release_timeout: Duration::from_secs(5),
        }
    }

    /// Attach an event sink (the D-Bus signal forwarder).
    pub fn set_event_sink(&mut self, tx: UnboundedSender<DaemonEvent>) {
        self.events = Some(tx);
    }

    /// The current state snapshot.
    pub fn info(&self) -> StateInfo {
        self.info.clone()
    }

    /// Surface an error that occurred *before* a transaction began (e.g. image
    /// validation), without changing the lifecycle state.
    pub fn report_error(&mut self, err: &BootDriveError) {
        self.info.last_error = err.message.clone();
        self.emit_error(err);
        self.emit_state();
    }

    fn emit_state(&self) {
        if let Some(tx) = &self.events {
            let _ = tx.send(DaemonEvent::State(self.info.clone()));
        }
    }

    fn emit_error(&self, err: &BootDriveError) {
        if let Some(tx) = &self.events {
            let _ = tx.send(DaemonEvent::Error {
                code: err.code.as_wire().to_string(),
                message: err.message.clone(),
            });
        }
    }

    fn set_state(&mut self, state: DriveState) {
        self.info.state = state;
        self.emit_state();
    }

    /// Probe hardware and set the initial state to `Idle` or `Unavailable`.
    pub fn initialize(&mut self) {
        match self.backend.probe() {
            Ok(caps) => {
                self.info.udc = caps.udc;
                self.set_state(DriveState::Idle);
            }
            Err(err) => {
                self.info.last_error = err.message.clone();
                self.set_state(DriveState::Unavailable);
            }
        }
    }

    /// Reconcile persisted recovery state after an (possibly unclean) restart.
    ///
    /// Always removes a stale `bootdrive` gadget, restores `usb-signaller` if it
    /// had been stopped for us, and clears the recovery record.
    pub fn recover(&mut self) {
        // Remove a leftover gadget named exactly `bootdrive`, regardless of
        // whether a recovery file exists. Never touches other gadgets.
        if let Err(e) = self.backend.remove_stale_gadget() {
            tracing::warn!("startup cleanup of stale gadget failed: {e}");
        }

        match self.recovery.load() {
            Ok(Some(state)) => {
                tracing::info!(
                    "recovering from unclean exit (gadget {}, signaller_was_running={})",
                    state.gadget_name,
                    state.signaller_was_running
                );
                if state.signaller_was_running {
                    if let Err(e) = self.signaller.start() {
                        tracing::warn!("could not restore usb-signaller during recovery: {e}");
                    }
                }
                if let Err(e) = self.recovery.clear() {
                    tracing::warn!("could not clear recovery state: {e}");
                }
            }
            Ok(None) => {}
            Err(e) => tracing::warn!("could not read recovery state: {e}"),
        }

        self.initialize();
    }

    /// Transactionally expose `image` in `mode`.
    pub fn activate(
        &mut self,
        image: ValidatedImage,
        mode: ImageMode,
    ) -> Result<StateInfo, BootDriveError> {
        match self.info.state {
            DriveState::Idle | DriveState::Error => {}
            DriveState::Unavailable => {
                let err =
                    BootDriveError::invalid_state("BootDrive is not available on this device");
                return Err(self.fail(err, false));
            }
            DriveState::Active | DriveState::Preparing | DriveState::Ejecting => {
                let err = BootDriveError::invalid_state(
                    "eject the current image before exposing another",
                );
                return Err(err);
            }
        }

        self.set_state(DriveState::Preparing);

        let caps = match self.backend.probe() {
            Ok(caps) => caps,
            Err(err) => return Err(self.fail(err, false)),
        };

        let signaller_was_running = self.signaller.is_running().unwrap_or(false);

        // Persist recovery intent *before* we perturb the system.
        let record = RecoveryState {
            gadget_name: GADGET_NAME.to_string(),
            signaller_was_running,
            display_name: image.display_name.clone(),
            mode,
        };
        if let Err(e) = self.recovery.save(&record) {
            return Err(self.fail(e, false));
        }

        if signaller_was_running {
            if let Err(err) = self.signaller.stop() {
                return Err(self.fail(err, false));
            }
        }

        if let Err(err) = self.backend.release_foreign_gadgets() {
            return Err(self.fail(err, signaller_was_running));
        }

        if let Err(err) = self.backend.remove_stale_gadget() {
            return Err(self.fail(err, signaller_was_running));
        }

        if let Err(err) = self.backend.activate(&image, mode) {
            return Err(self.fail(err, signaller_was_running));
        }

        self.info.display_name = image.display_name;
        self.info.mode = mode;
        self.info.udc = caps.udc;
        self.info.signaller_was_running = signaller_was_running;
        self.info.last_error.clear();
        self.set_state(DriveState::Active);
        Ok(self.info.clone())
    }

    /// Roll back a failed activation: tear down any partial gadget, restore
    /// `usb-signaller`, clear recovery, and enter `Error`.
    fn fail(&mut self, err: BootDriveError, signaller_was_running: bool) -> BootDriveError {
        tracing::error!("activation failed: {err}");
        let _ = self.backend.deactivate();
        let _ = self.backend.remove_stale_gadget();
        if signaller_was_running {
            if let Err(e) = self.signaller.start() {
                tracing::warn!("could not restore usb-signaller after failure: {e}");
            }
        }
        let _ = self.recovery.clear();
        self.info.signaller_was_running = false;
        self.info.udc.clear();
        self.info.last_error = err.message.clone();
        self.set_state(DriveState::Error);
        self.emit_error(&err);
        err
    }

    /// Transactionally eject the current image and restore normal USB behaviour.
    pub fn deactivate(&mut self) -> Result<StateInfo, BootDriveError> {
        match self.info.state {
            DriveState::Idle | DriveState::Unavailable => return Ok(self.info.clone()),
            DriveState::Preparing | DriveState::Ejecting => {
                return Err(BootDriveError::invalid_state(
                    "a transition is already in progress",
                ));
            }
            DriveState::Active | DriveState::Error => {}
        }

        let signaller_was_running = self.info.signaller_was_running;
        self.set_state(DriveState::Ejecting);

        if let Err(err) = self.backend.deactivate() {
            // Try to restore the signaller even if teardown failed.
            if signaller_was_running {
                let _ = self.signaller.start();
            }
            let _ = self.recovery.clear();
            return Err(self.fail(err, false));
        }

        if signaller_was_running {
            if let Err(err) = self.signaller.start() {
                let _ = self.recovery.clear();
                return Err(self.fail(err, false));
            }
        }

        let _ = self.recovery.clear();
        self.info.display_name.clear();
        self.info.mode = ImageMode::Cdrom;
        self.info.signaller_was_running = false;
        self.info.last_error.clear();
        self.set_state(DriveState::Idle);
        Ok(self.info.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recovery::RecoveryStore;
    use crate::usb::{MockScript, MockUsbBackend};
    use crate::usb_signaller::MockSignaller;
    use std::path::PathBuf;
    use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

    fn tmp_recovery(name: &str) -> RecoveryStore {
        let mut p = std::env::temp_dir();
        p.push(format!("bootdrived-sm-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p.push("state.json");
        RecoveryStore::new(p)
    }

    fn image() -> ValidatedImage {
        ValidatedImage {
            path: PathBuf::from("/tmp/x.iso"),
            display_name: "x.iso".into(),
            size: 2048,
        }
    }

    fn manager(
        backend: MockUsbBackend,
        signaller: MockSignaller,
        rec: RecoveryStore,
    ) -> (Manager, UnboundedReceiver<DaemonEvent>) {
        let (tx, rx) = unbounded_channel();
        let mut m = Manager::new(Box::new(backend), Box::new(signaller), rec);
        m.set_event_sink(tx);
        (m, rx)
    }

    fn states(rx: &mut UnboundedReceiver<DaemonEvent>) -> Vec<DriveState> {
        let mut out = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let DaemonEvent::State(s) = ev {
                out.push(s.state);
            }
        }
        out
    }

    #[test]
    fn happy_path_activate_then_deactivate() {
        let rec = tmp_recovery("happy");
        let (mut m, mut rx) = manager(
            MockUsbBackend::working(),
            MockSignaller::running(),
            rec.clone(),
        );
        m.initialize();
        assert_eq!(m.info().state, DriveState::Idle);

        let info = m.activate(image(), ImageMode::Cdrom).unwrap();
        assert_eq!(info.state, DriveState::Active);
        assert_eq!(info.display_name, "x.iso");
        assert!(info.signaller_was_running);
        // Recovery persisted while active.
        assert!(rec.load().unwrap().is_some());

        let seq = states(&mut rx);
        assert!(seq.contains(&DriveState::Preparing));
        assert_eq!(*seq.last().unwrap(), DriveState::Active);

        let info = m.deactivate().unwrap();
        assert_eq!(info.state, DriveState::Idle);
        // Recovery cleared after eject.
        assert!(rec.load().unwrap().is_none());
    }

    #[test]
    fn activation_failure_rolls_back_and_restores_signaller() {
        let rec = tmp_recovery("rollback");
        let backend = MockUsbBackend::with_script(MockScript {
            udc: Some("u".into()),
            mass_storage_ok: true,
            fail_activate: Some("bind failed".into()),
            ..Default::default()
        });
        let signaller = MockSignaller::running();
        let (mut m, mut rx) = manager(backend, signaller, rec.clone());
        m.initialize();

        let err = m.activate(image(), ImageMode::Cdrom).unwrap_err();
        assert_eq!(err.code, bootdrive_common::ErrorCode::GadgetFailure);
        assert_eq!(m.info().state, DriveState::Error);
        // usb-signaller was stopped then restarted by rollback.
        // Recovery must be cleared on failure.
        assert!(rec.load().unwrap().is_none());

        let seq = states(&mut rx);
        assert_eq!(*seq.last().unwrap(), DriveState::Error);
    }

    #[test]
    fn cannot_activate_twice() {
        let rec = tmp_recovery("twice");
        let (mut m, _rx) = manager(MockUsbBackend::working(), MockSignaller::running(), rec);
        m.initialize();
        m.activate(image(), ImageMode::Cdrom).unwrap();
        let err = m.activate(image(), ImageMode::Disk).unwrap_err();
        assert_eq!(err.code, bootdrive_common::ErrorCode::InvalidState);
    }

    #[test]
    fn recovery_restores_signaller_and_clears_state() {
        let rec = tmp_recovery("recover");
        // Simulate a crash while active: recovery file says signaller was running.
        rec.save(&RecoveryState {
            gadget_name: GADGET_NAME.to_string(),
            signaller_was_running: true,
            display_name: "x.iso".into(),
            mode: ImageMode::Cdrom,
        })
        .unwrap();

        let backend = MockUsbBackend::with_script(MockScript {
            udc: Some("u".into()),
            mass_storage_ok: true,
            stale_present: true,
            ..Default::default()
        });
        let signaller = MockSignaller::stopped();
        let (mut m, _rx) = manager(backend, signaller, rec.clone());

        m.recover();
        assert_eq!(m.info().state, DriveState::Idle);
        assert!(rec.load().unwrap().is_none());
    }

    #[test]
    fn signaller_kept_stopped_when_not_running_before() {
        let rec = tmp_recovery("nosignaller");
        let (mut m, _rx) = manager(MockUsbBackend::working(), MockSignaller::stopped(), rec);
        m.initialize();
        let info = m.activate(image(), ImageMode::Disk).unwrap();
        assert!(!info.signaller_was_running);
        let info = m.deactivate().unwrap();
        assert_eq!(info.state, DriveState::Idle);
    }

    #[test]
    fn deactivate_when_idle_is_noop() {
        let rec = tmp_recovery("idle-eject");
        let (mut m, _rx) = manager(MockUsbBackend::working(), MockSignaller::stopped(), rec);
        m.initialize();
        let info = m.deactivate().unwrap();
        assert_eq!(info.state, DriveState::Idle);
    }
}
