//! The GUI's view of `bootdrived` over the **system** bus.
//!
//! A dedicated Tokio runtime on a background thread runs the async zbus proxy
//! and bridges to the GTK main loop with async channels, so the GUI thread
//! never blocks on D-Bus.

use std::sync::Arc;
use std::thread;

use bootdrive_common::{DriveState, ImageMode, StateInfo};
use zbus::proxy;

/// Update pushed from the D-Bus thread to the GTK thread.
#[derive(Debug, Clone)]
pub enum DaemonUpdate {
    /// A fresh state snapshot (initial fetch or `StateChanged`).
    State(StateInfo),
    /// The daemon reported (or a call returned) an error.
    Error {
        /// Stable machine-readable code.
        code: String,
        /// Human-readable message.
        message: String,
    },
    /// The daemon is unreachable (not installed / not running / no permission).
    Unreachable(UnreachableReason),
}

/// Why the daemon could not be reached, mapped to a GUI setup hint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnreachableReason {
    /// The well-known name has no owner (service not installed / not started).
    NotRunning,
    /// The bus denied access to the name/method.
    PermissionDenied,
    /// Any other connection failure.
    Other(String),
}

/// Command pushed from the GTK thread to the D-Bus thread.
#[derive(Debug, Clone)]
pub enum DaemonCommand {
    /// Ask the daemon to expose an image.
    Activate {
        /// Resolved host path.
        path: String,
        /// GUI-safe display name.
        display_name: String,
        /// Exposure mode.
        mode: ImageMode,
    },
    /// Ask the daemon to eject.
    Deactivate,
}

#[proxy(
    interface = "net.bresilla.BootDrive1",
    default_service = "net.bresilla.BootDrive1",
    default_path = "/net/bresilla/BootDrive1"
)]
trait BootDrive {
    fn get_state(&self) -> zbus::Result<StateInfo>;
    fn activate(&self, image_path: &str, display_name: &str, mode: &str) -> zbus::Result<()>;
    fn deactivate(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn state_changed(&self, info: StateInfo) -> zbus::Result<()>;
    #[zbus(signal)]
    fn error_occurred(&self, code: &str, message: &str) -> zbus::Result<()>;
}

/// Handle held by the GTK side.
pub struct DaemonClient {
    /// Send commands to the D-Bus thread.
    pub commands: async_channel::Sender<DaemonCommand>,
    /// Receive updates on the GTK main loop.
    pub updates: async_channel::Receiver<DaemonUpdate>,
}

impl DaemonClient {
    /// Spawn the background D-Bus thread and return a client handle.
    pub fn spawn() -> DaemonClient {
        let (cmd_tx, cmd_rx) = async_channel::unbounded::<DaemonCommand>();
        let (upd_tx, upd_rx) = async_channel::unbounded::<DaemonUpdate>();

        thread::Builder::new()
            .name("bootdrive-dbus".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = upd_tx.send_blocking(DaemonUpdate::Unreachable(
                            UnreachableReason::Other(format!("runtime error: {e}")),
                        ));
                        return;
                    }
                };
                rt.block_on(dbus_worker(cmd_rx, upd_tx));
            })
            .expect("spawn D-Bus thread");

        DaemonClient {
            commands: cmd_tx,
            updates: upd_rx,
        }
    }
}

/// Classify a zbus error into an [`UnreachableReason`].
fn classify(err: &zbus::Error) -> UnreachableReason {
    match err {
        zbus::Error::MethodError(name, _, _) => {
            let n = name.as_str();
            if n.contains("AccessDenied") || n.contains("NotAuthorized") {
                UnreachableReason::PermissionDenied
            } else if n.contains("ServiceUnknown") || n.contains("NameHasNoOwner") {
                UnreachableReason::NotRunning
            } else {
                UnreachableReason::Other(n.to_string())
            }
        }
        other => UnreachableReason::Other(other.to_string()),
    }
}

/// Split a `"code: message"` method error back into its parts.
fn split_coded(err: &zbus::Error) -> (String, String) {
    if let zbus::Error::MethodError(_, Some(text), _) = err {
        if let Some((code, msg)) = text.split_once(": ") {
            return (code.to_string(), msg.to_string());
        }
        return ("internal".into(), text.clone());
    }
    ("internal".into(), err.to_string())
}

async fn dbus_worker(
    commands: async_channel::Receiver<DaemonCommand>,
    updates: async_channel::Sender<DaemonUpdate>,
) {
    let conn = match zbus::Connection::system().await {
        Ok(c) => c,
        Err(e) => {
            let _ = updates
                .send(DaemonUpdate::Unreachable(UnreachableReason::Other(
                    format!("no system bus: {e}"),
                )))
                .await;
            return;
        }
    };

    let proxy = match BootDriveProxy::new(&conn).await {
        Ok(p) => p,
        Err(e) => {
            let _ = updates.send(DaemonUpdate::Unreachable(classify(&e))).await;
            return;
        }
    };

    // Initial state fetch. A failure usually means the daemon is absent.
    match proxy.get_state().await {
        Ok(state) => {
            let _ = updates.send(DaemonUpdate::State(state)).await;
        }
        Err(e) => {
            let _ = updates.send(DaemonUpdate::Unreachable(classify(&e))).await;
        }
    }

    let proxy = Arc::new(proxy);

    // Forward StateChanged signals.
    let state_updates = updates.clone();
    let state_proxy = proxy.clone();
    tokio::spawn(async move {
        if let Ok(mut stream) = state_proxy.receive_state_changed().await {
            use futures_util::StreamExt;
            while let Some(sig) = stream.next().await {
                if let Ok(args) = sig.args() {
                    let _ = state_updates.send(DaemonUpdate::State(args.info)).await;
                }
            }
        }
    });

    // Forward ErrorOccurred signals.
    let err_updates = updates.clone();
    let err_proxy = proxy.clone();
    tokio::spawn(async move {
        if let Ok(mut stream) = err_proxy.receive_error_occurred().await {
            use futures_util::StreamExt;
            while let Some(sig) = stream.next().await {
                if let Ok(args) = sig.args() {
                    let _ = err_updates
                        .send(DaemonUpdate::Error {
                            code: args.code.to_string(),
                            message: args.message.to_string(),
                        })
                        .await;
                }
            }
        }
    });

    // Command loop. Method failures are surfaced as Error updates; success is
    // reflected by the subsequent StateChanged signal.
    while let Ok(cmd) = commands.recv().await {
        let result = match cmd {
            DaemonCommand::Activate {
                path,
                display_name,
                mode,
            } => proxy.activate(&path, &display_name, mode.as_wire()).await,
            DaemonCommand::Deactivate => proxy.deactivate().await,
        };
        if let Err(e) = result {
            let (code, message) = split_coded(&e);
            let _ = updates.send(DaemonUpdate::Error { code, message }).await;
        }
    }
}

/// A user-facing headline for a state.
pub fn headline(state: DriveState) -> &'static str {
    match state {
        DriveState::Unavailable => "Unavailable",
        DriveState::Idle => "Ready",
        DriveState::Preparing => "Preparing…",
        DriveState::Active => "Active",
        DriveState::Ejecting => "Ejecting…",
        DriveState::Error => "Error",
    }
}
