//! The GUI's connection to `usb-signaller` over `com.meego.usb_moded`.
//!
//! A dedicated Tokio runtime on a background thread runs the async zbus proxy
//! and bridges to the GTK main loop with async channels. To expose an image we
//! `set_config("image=…,cdrom=…")` then `set_mode("mass_storage_mode")`; to
//! eject we `set_mode("developer_mode")`.

use std::sync::Arc;
use std::thread;

use bootdrive_common::{DriveState, ImageMode, MODE_MASS_STORAGE, MODE_NORMAL};
use zbus::proxy;

/// Update pushed from the D-Bus thread to the GTK thread.
#[derive(Debug, Clone)]
pub enum DaemonUpdate {
    /// A fresh state (derived from the current USB mode).
    State(DriveState),
    /// An operation failed.
    Error {
        /// Human-readable message.
        message: String,
    },
    /// usb-signaller is unreachable or lacks mass-storage support.
    Unreachable(UnreachableReason),
}

/// Why the service is unavailable, mapped to a GUI setup hint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnreachableReason {
    /// usb-signaller is not running / not reachable.
    NotRunning,
    /// usb-signaller is running but has no `mass_storage_mode` (patch missing).
    NoMassStorage,
    /// Any other failure.
    Other(String),
}

/// Command pushed from the GTK thread to the D-Bus thread.
#[derive(Debug, Clone)]
pub enum DaemonCommand {
    /// Expose an image.
    Activate {
        /// Resolved host path.
        path: String,
        /// Exposure mode.
        mode: ImageMode,
    },
    /// Eject (return to normal mode).
    Deactivate,
}

#[proxy(
    interface = "com.meego.usb_moded",
    default_service = "com.meego.usb_moded",
    default_path = "/com/meego/usb_moded"
)]
trait UsbModed {
    // usb-signaller uses snake_case D-Bus method names; pin them so zbus does
    // not PascalCase them (which yields UnknownMethod).
    #[zbus(name = "get_modes")]
    fn get_modes(&self) -> zbus::Result<String>;
    #[zbus(name = "mode_request")]
    fn mode_request(&self) -> zbus::Result<String>;
    #[zbus(name = "set_mode")]
    fn set_mode(&self, mode: &str) -> zbus::Result<String>;
    #[zbus(name = "set_config")]
    fn set_config(&self, config: &str) -> zbus::Result<String>;

    #[zbus(signal, name = "sig_usb_event_ind")]
    fn sig_usb_event_ind(&self, event: String) -> zbus::Result<()>;
}

/// Handle held by the GTK side.
pub struct DaemonClient {
    /// Send commands to the D-Bus thread.
    pub commands: async_channel::Sender<DaemonCommand>,
    /// Receive updates on the GTK main loop.
    pub updates: async_channel::Receiver<DaemonUpdate>,
}

/// Map a USB mode string to our display state.
fn state_from_mode(mode: &str) -> DriveState {
    if mode == MODE_MASS_STORAGE {
        DriveState::Active
    } else {
        DriveState::Idle
    }
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
                rt.block_on(worker(cmd_rx, upd_tx));
            })
            .expect("spawn D-Bus thread");

        DaemonClient {
            commands: cmd_tx,
            updates: upd_rx,
        }
    }
}

async fn worker(
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

    // usb-signaller's D-Bus policy blocks org.freedesktop.DBus.Properties, so
    // we must NOT let zbus cache properties (it would call GetAll and be
    // denied). Disable caching; we only use plain methods and a signal.
    let proxy = match UsbModedProxy::builder(&conn)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
    {
        Ok(p) => p,
        Err(e) => {
            let _ = updates
                .send(DaemonUpdate::Unreachable(UnreachableReason::Other(
                    e.to_string(),
                )))
                .await;
            return;
        }
    };

    // Require mass-storage support (our patch), else prompt to install it.
    match proxy.get_modes().await {
        Ok(modes) if modes.split(',').any(|m| m == MODE_MASS_STORAGE) => {}
        Ok(_) => {
            let _ = updates
                .send(DaemonUpdate::Unreachable(UnreachableReason::NoMassStorage))
                .await;
            return;
        }
        Err(_) => {
            let _ = updates
                .send(DaemonUpdate::Unreachable(UnreachableReason::NotRunning))
                .await;
            return;
        }
    }

    if let Ok(mode) = proxy.mode_request().await {
        let _ = updates
            .send(DaemonUpdate::State(state_from_mode(&mode)))
            .await;
    }

    let proxy = Arc::new(proxy);

    // Follow mode changes.
    let sig_updates = updates.clone();
    let sig_proxy = proxy.clone();
    tokio::spawn(async move {
        if let Ok(mut stream) = sig_proxy.receive_sig_usb_event_ind().await {
            use futures_util::StreamExt;
            while stream.next().await.is_some() {
                if let Ok(mode) = sig_proxy.mode_request().await {
                    let _ = sig_updates
                        .send(DaemonUpdate::State(state_from_mode(&mode)))
                        .await;
                }
            }
        }
    });

    // Command loop.
    while let Ok(cmd) = commands.recv().await {
        match cmd {
            DaemonCommand::Activate { path, mode } => {
                let config = format!("image={path},cdrom={}", mode.cdrom_flag());
                if let Err(e) = proxy.set_config(&config).await {
                    let _ = updates
                        .send(DaemonUpdate::Error {
                            message: format!("could not set image: {e}"),
                        })
                        .await;
                    continue;
                }
                match proxy.set_mode(MODE_MASS_STORAGE).await {
                    Ok(result) if result == MODE_MASS_STORAGE => {
                        let _ = updates.send(DaemonUpdate::State(DriveState::Active)).await;
                    }
                    Ok(other) => {
                        let _ = updates
                            .send(DaemonUpdate::Error {
                                message: format!("USB stayed in '{other}', check usb-signaller"),
                            })
                            .await;
                    }
                    Err(e) => {
                        let _ = updates
                            .send(DaemonUpdate::Error {
                                message: format!("could not expose the image: {e}"),
                            })
                            .await;
                    }
                }
            }
            DaemonCommand::Deactivate => match proxy.set_mode(MODE_NORMAL).await {
                Ok(_) => {
                    let _ = updates.send(DaemonUpdate::State(DriveState::Idle)).await;
                }
                Err(e) => {
                    let _ = updates
                        .send(DaemonUpdate::Error {
                            message: format!("could not eject: {e}"),
                        })
                        .await;
                }
            },
        }
    }
}
