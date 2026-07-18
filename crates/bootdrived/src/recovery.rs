//! Minimal, crash-safe recovery state.
//!
//! When BootDrive is active it records just enough under
//! `/run/bootdrived/state.json` (runtime, not configuration) to undo an
//! activation after an unclean exit: whether `usb-signaller` was running and
//! must be restored, and the name of the gadget we own so startup cleanup only
//! ever targets `bootdrive` (plan section 12).

use std::path::{Path, PathBuf};

use bootdrive_common::{BootDriveError, ImageMode};
use serde::{Deserialize, Serialize};

/// Default runtime location of the recovery file.
pub const DEFAULT_STATE_PATH: &str = "/run/bootdrived/state.json";

/// Persisted recovery record. Absent file == nothing to recover.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryState {
    /// The gadget BootDrive created; only this name is ever cleaned up.
    pub gadget_name: String,
    /// Whether `usb-signaller` was running before activation.
    pub signaller_was_running: bool,
    /// Display name of the exposed image (for logging/UX only).
    pub display_name: String,
    /// The mode the image was exposed in.
    pub mode: ImageMode,
}

/// A filesystem-backed store for [`RecoveryState`], parameterized on its path so
/// tests can point it at a temp dir.
#[derive(Debug, Clone)]
pub struct RecoveryStore {
    path: PathBuf,
}

impl Default for RecoveryStore {
    fn default() -> Self {
        RecoveryStore::new(DEFAULT_STATE_PATH)
    }
}

impl RecoveryStore {
    /// Create a store writing to `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        RecoveryStore { path: path.into() }
    }

    /// The path this store persists to.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load the current recovery record, if any. A missing file is `Ok(None)`;
    /// a corrupt file is treated as `Ok(None)` (best-effort) after a warning.
    pub fn load(&self) -> Result<Option<RecoveryState>, BootDriveError> {
        match std::fs::read(&self.path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(state) => Ok(Some(state)),
                Err(e) => {
                    tracing::warn!("ignoring corrupt recovery state: {e}");
                    Ok(None)
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(BootDriveError::internal(format!(
                "could not read recovery state: {e}"
            ))),
        }
    }

    /// Persist a recovery record, creating the parent directory if needed.
    pub fn save(&self, state: &RecoveryState) -> Result<(), BootDriveError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                BootDriveError::internal(format!("could not create runtime dir: {e}"))
            })?;
        }
        let json = serde_json::to_vec_pretty(state)
            .map_err(|e| BootDriveError::internal(format!("could not encode recovery: {e}")))?;
        // Write to a temp file then rename for atomicity.
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, &json)
            .map_err(|e| BootDriveError::internal(format!("could not write recovery: {e}")))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| BootDriveError::internal(format!("could not commit recovery: {e}")))?;
        Ok(())
    }

    /// Remove the recovery record (idempotent).
    pub fn clear(&self) -> Result<(), BootDriveError> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(BootDriveError::internal(format!(
                "could not clear recovery state: {e}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_store(name: &str) -> RecoveryStore {
        let mut p = std::env::temp_dir();
        p.push(format!("bootdrived-test-{}-{}", std::process::id(), name));
        std::fs::create_dir_all(&p).unwrap();
        p.push("state.json");
        let _ = std::fs::remove_file(&p);
        RecoveryStore::new(p)
    }

    fn sample() -> RecoveryState {
        RecoveryState {
            gadget_name: "bootdrive".into(),
            signaller_was_running: true,
            display_name: "debian.iso".into(),
            mode: ImageMode::Cdrom,
        }
    }

    #[test]
    fn missing_file_is_none() {
        let store = tmp_store("missing");
        assert_eq!(store.load().unwrap(), None);
    }

    #[test]
    fn save_load_round_trip() {
        let store = tmp_store("roundtrip");
        store.save(&sample()).unwrap();
        assert_eq!(store.load().unwrap(), Some(sample()));
    }

    #[test]
    fn clear_is_idempotent() {
        let store = tmp_store("clear");
        store.save(&sample()).unwrap();
        store.clear().unwrap();
        assert_eq!(store.load().unwrap(), None);
        store.clear().unwrap();
    }

    #[test]
    fn corrupt_file_is_ignored() {
        let store = tmp_store("corrupt");
        std::fs::write(store.path(), b"{ not json").unwrap();
        assert_eq!(store.load().unwrap(), None);
    }
}
