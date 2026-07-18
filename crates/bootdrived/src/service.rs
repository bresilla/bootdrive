//! The system-D-Bus adapter around [`Manager`].
//!
//! Thin by design: it resolves the caller's identity from bus credentials,
//! authorizes, validates the selected path, then delegates the transaction to
//! [`Manager`]. State changes flow back out as signals via a forwarder task fed
//! by the manager's event channel, so the interface methods never emit signals
//! themselves.

use std::sync::{Arc, Mutex};

use bootdrive_common::{BootDriveError, ErrorCode, ImageMode, StateInfo};
use tokio::sync::mpsc::UnboundedReceiver;
use zbus::object_server::SignalEmitter;
use zbus::{fdo, interface, Connection};

use crate::authorization::Authorizer;
use crate::caller;
use crate::image::{self, ValidatedImage};
use crate::state::{DaemonEvent, Manager};

/// Shared, thread-safe handle to the state machine.
pub type SharedManager = Arc<Mutex<Manager>>;

/// The D-Bus object implementing `net.bresilla.BootDrive1`.
pub struct BootDriveService {
    manager: SharedManager,
    authorizer: Arc<dyn Authorizer>,
}

impl BootDriveService {
    /// Create the service object.
    pub fn new(manager: SharedManager, authorizer: Arc<dyn Authorizer>) -> Self {
        BootDriveService {
            manager,
            authorizer,
        }
    }
}

/// Map a domain error to a D-Bus error, preserving the stable code in the text
/// so a client that only reads the reply still recovers it.
fn to_fdo(err: &BootDriveError) -> fdo::Error {
    let text = format!("{}: {}", err.code.as_wire(), err.message);
    match err.code {
        ErrorCode::NotAuthorized => fdo::Error::AccessDenied(text),
        ErrorCode::InvalidImage | ErrorCode::InvalidState => fdo::Error::InvalidArgs(text),
        _ => fdo::Error::Failed(text),
    }
}

/// Resolve the caller uid from bus credentials — never from a parameter.
async fn caller_uid(conn: &Connection, hdr: &zbus::message::Header<'_>) -> Result<u32, fdo::Error> {
    let sender = hdr
        .sender()
        .ok_or_else(|| fdo::Error::Failed("could not determine D-Bus caller".into()))?;
    let dbus = fdo::DBusProxy::new(conn).await?;
    let uid = dbus
        .get_connection_unix_user(sender.to_owned().into())
        .await?;
    Ok(uid)
}

#[interface(name = "net.bresilla.BootDrive1")]
impl BootDriveService {
    /// Return the current full state snapshot.
    async fn get_state(&self) -> StateInfo {
        self.manager.lock().unwrap().info()
    }

    /// Expose `image_path` (a resolved host path) as `mode`.
    async fn activate(
        &self,
        image_path: &str,
        display_name: &str,
        mode: &str,
        #[zbus(connection)] conn: &Connection,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> fdo::Result<()> {
        let uid = caller_uid(conn, &hdr).await?;
        let who = caller::from_uid(uid);

        self.authorizer
            .authorize_activate(&who)
            .map_err(|e| to_fdo(&e))?;

        let mode = mode.parse::<ImageMode>().unwrap_or_else(|_| {
            let ext = std::path::Path::new(image_path)
                .extension()
                .map(|e| e.to_string_lossy().into_owned())
                .unwrap_or_default();
            ImageMode::default_for_extension(&ext)
        });

        let path = std::path::PathBuf::from(image_path);
        let validated: ValidatedImage =
            image::validate(&path, display_name, &who).map_err(|e| to_fdo(&e))?;

        tracing::info!(
            "activate requested by uid {uid}: '{}' as {}",
            validated.display_name,
            mode.label()
        );

        let manager = self.manager.clone();
        let res =
            tokio::task::spawn_blocking(move || manager.lock().unwrap().activate(validated, mode))
                .await
                .map_err(|e| fdo::Error::Failed(format!("internal task failure: {e}")))?;

        res.map(|_| ()).map_err(|e| to_fdo(&e))
    }

    /// Eject the current image and restore normal USB behaviour.
    async fn deactivate(
        &self,
        #[zbus(connection)] conn: &Connection,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> fdo::Result<()> {
        let uid = caller_uid(conn, &hdr).await?;
        let who = caller::from_uid(uid);
        self.authorizer
            .authorize_activate(&who)
            .map_err(|e| to_fdo(&e))?;

        tracing::info!("deactivate requested by uid {uid}");

        let manager = self.manager.clone();
        let res = tokio::task::spawn_blocking(move || manager.lock().unwrap().deactivate())
            .await
            .map_err(|e| fdo::Error::Failed(format!("internal task failure: {e}")))?;

        res.map(|_| ()).map_err(|e| to_fdo(&e))
    }

    /// Emitted whenever the state changes.
    #[zbus(signal)]
    async fn state_changed(emitter: &SignalEmitter<'_>, info: StateInfo) -> zbus::Result<()>;

    /// Emitted when an operation fails, carrying a stable machine-readable code.
    #[zbus(signal)]
    async fn error_occurred(
        emitter: &SignalEmitter<'_>,
        code: &str,
        message: &str,
    ) -> zbus::Result<()>;
}

/// Forward [`DaemonEvent`]s from the manager onto the bus as signals.
pub async fn run_event_forwarder(conn: Connection, mut rx: UnboundedReceiver<DaemonEvent>) {
    let emitter = match SignalEmitter::new(&conn, bootdrive_common::DBUS_OBJECT_PATH) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("could not create signal emitter: {e}");
            return;
        }
    };

    while let Some(event) = rx.recv().await {
        match event {
            DaemonEvent::State(info) => {
                if let Err(e) = BootDriveService::state_changed(&emitter, info).await {
                    tracing::warn!("failed to emit StateChanged: {e}");
                }
            }
            DaemonEvent::Error { code, message } => {
                if let Err(e) = BootDriveService::error_occurred(&emitter, &code, &message).await {
                    tracing::warn!("failed to emit ErrorOccurred: {e}");
                }
            }
        }
    }
}
