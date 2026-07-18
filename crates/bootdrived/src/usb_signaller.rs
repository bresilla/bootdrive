//! Coordination with postmarketOS's `usb-signaller` service.
//!
//! `usb-signaller` normally owns gadget `g1` and the UDC. BootDrive must borrow
//! the UDC while active and hand it back afterwards. This module wraps the init
//! system behind [`SignallerControl`] so the handoff logic in [`crate::state`]
//! can be tested with [`MockSignaller`] and no real service.
//!
//! postmarketOS ships with either systemd or OpenRC depending on the install,
//! so [`detect`] picks the right controller at runtime by probing
//! `/run/systemd/system`. Absolute binary paths are used so behaviour does not
//! depend on the helper's `PATH`.

use std::process::Command;
use std::time::{Duration, Instant};

use bootdrive_common::{BootDriveError, ErrorCode};

const SERVICE_NAME: &str = "usb-signaller";
const SYSTEMCTL: &str = "/usr/bin/systemctl";
const RC_SERVICE: &str = "/sbin/rc-service";

/// Control surface for the normal USB gadget owner.
pub trait SignallerControl: Send {
    /// Whether the service is currently running.
    fn is_running(&self) -> Result<bool, BootDriveError>;
    /// Stop the service.
    fn stop(&self) -> Result<(), BootDriveError>;
    /// Start the service.
    fn start(&self) -> Result<(), BootDriveError>;
}

/// Pick the correct controller for the running init system.
pub fn detect() -> Box<dyn SignallerControl> {
    if std::path::Path::new("/run/systemd/system").is_dir() {
        tracing::debug!("using systemd to control usb-signaller");
        Box::new(SystemdSignaller::new())
    } else {
        tracing::debug!("using OpenRC to control usb-signaller");
        Box::new(OpenRcSignaller::new())
    }
}

fn signal_error(action: &str, detail: impl std::fmt::Display) -> BootDriveError {
    BootDriveError::new(
        ErrorCode::SignallerReleaseFailed,
        format!("usb-signaller could not be {action}: {detail}"),
    )
}

/// systemd-backed control (`systemctl <action> usb-signaller.service`).
#[derive(Debug, Default)]
pub struct SystemdSignaller;

impl SystemdSignaller {
    /// Create a new controller.
    pub fn new() -> Self {
        SystemdSignaller
    }

    fn run(&self, action: &str) -> Result<std::process::Output, BootDriveError> {
        Command::new(SYSTEMCTL)
            .arg(action)
            .arg(SERVICE_NAME)
            .output()
            .map_err(|e| signal_error(action, e))
    }
}

impl SignallerControl for SystemdSignaller {
    fn is_running(&self) -> Result<bool, BootDriveError> {
        // `systemctl is-active` exits 0 and prints "active" when running.
        let out = self.run("is-active")?;
        Ok(out.status.success())
    }

    fn stop(&self) -> Result<(), BootDriveError> {
        let out = self.run("stop")?;
        if out.status.success() {
            Ok(())
        } else {
            Err(signal_error(
                "stopped",
                String::from_utf8_lossy(&out.stderr).trim(),
            ))
        }
    }

    fn start(&self) -> Result<(), BootDriveError> {
        let out = self.run("start")?;
        if out.status.success() {
            Ok(())
        } else {
            Err(signal_error(
                "restarted",
                String::from_utf8_lossy(&out.stderr).trim(),
            ))
        }
    }
}

/// OpenRC-backed control (`rc-service usb-signaller <action>`).
#[derive(Debug, Default)]
pub struct OpenRcSignaller;

impl OpenRcSignaller {
    /// Create a new controller.
    pub fn new() -> Self {
        OpenRcSignaller
    }

    fn run(&self, action: &str) -> Result<std::process::Output, BootDriveError> {
        Command::new(RC_SERVICE)
            .arg(SERVICE_NAME)
            .arg(action)
            .output()
            .map_err(|e| signal_error(action, e))
    }
}

impl SignallerControl for OpenRcSignaller {
    fn is_running(&self) -> Result<bool, BootDriveError> {
        let out = self.run("status")?;
        Ok(out.status.success())
    }

    fn stop(&self) -> Result<(), BootDriveError> {
        let out = self.run("stop")?;
        if out.status.success() {
            Ok(())
        } else {
            Err(signal_error(
                "stopped",
                String::from_utf8_lossy(&out.stderr).trim(),
            ))
        }
    }

    fn start(&self) -> Result<(), BootDriveError> {
        let out = self.run("start")?;
        if out.status.success() {
            Ok(())
        } else {
            Err(signal_error(
                "restarted",
                String::from_utf8_lossy(&out.stderr).trim(),
            ))
        }
    }
}

/// Poll `is_bound` until the UDC is released, up to `timeout`. `is_bound` should
/// report whether *any foreign* gadget still holds the UDC.
pub fn wait_for_release<F>(mut is_bound: F, timeout: Duration) -> Result<(), BootDriveError>
where
    F: FnMut() -> bool,
{
    let deadline = Instant::now() + timeout;
    loop {
        if !is_bound() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(BootDriveError::new(
                ErrorCode::UdcBusy,
                "the UDC was not released in time",
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Test double.
#[derive(Debug, Default)]
pub struct MockSignaller {
    /// Whether the service reports running.
    pub running: std::cell::Cell<bool>,
    /// If set, `stop` fails.
    pub fail_stop: bool,
    /// Record of actions taken, for assertions.
    pub actions: std::cell::RefCell<Vec<String>>,
}

impl MockSignaller {
    /// Create a mock that is currently running.
    pub fn running() -> Self {
        MockSignaller {
            running: std::cell::Cell::new(true),
            ..Default::default()
        }
    }

    /// Create a mock that is currently stopped.
    pub fn stopped() -> Self {
        MockSignaller {
            running: std::cell::Cell::new(false),
            ..Default::default()
        }
    }
}

impl SignallerControl for MockSignaller {
    fn is_running(&self) -> Result<bool, BootDriveError> {
        Ok(self.running.get())
    }

    fn stop(&self) -> Result<(), BootDriveError> {
        self.actions.borrow_mut().push("stop".into());
        if self.fail_stop {
            return Err(BootDriveError::new(
                ErrorCode::SignallerReleaseFailed,
                "mock stop failure",
            ));
        }
        self.running.set(false);
        Ok(())
    }

    fn start(&self) -> Result<(), BootDriveError> {
        self.actions.borrow_mut().push("start".into());
        self.running.set(true);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_returns_when_released_immediately() {
        assert!(wait_for_release(|| false, Duration::from_millis(500)).is_ok());
    }

    #[test]
    fn wait_times_out_when_never_released() {
        let err = wait_for_release(|| true, Duration::from_millis(150)).unwrap_err();
        assert_eq!(err.code, ErrorCode::UdcBusy);
    }

    #[test]
    fn mock_stop_and_start_toggle_state() {
        let s = MockSignaller::running();
        assert!(s.is_running().unwrap());
        s.stop().unwrap();
        assert!(!s.is_running().unwrap());
        s.start().unwrap();
        assert!(s.is_running().unwrap());
        assert_eq!(*s.actions.borrow(), vec!["stop", "start"]);
    }
}
