//! The `bootdrived` service entry point.
//!
//! Wires the real backends together, runs startup recovery, claims
//! `net.bresilla.BootDrive1` on the **system** bus, and serves until a
//! termination signal. Lifecycle follows the service: an exposed image stays up
//! until a frontend ejects it — not tied to any frontend being open.

use std::sync::{Arc, Mutex};

use bootdrive_common::{DBUS_OBJECT_PATH, DBUS_SERVICE_NAME};
use bootdrived::authorization::{AllowAllAuthorizer, Authorizer, GroupAuthorizer};
use bootdrived::recovery::RecoveryStore;
use bootdrived::service::{run_event_forwarder, BootDriveService};
use bootdrived::state::Manager;
use bootdrived::usb::UsbGadgetBackend;
use bootdrived::usb_signaller;
use tokio::sync::mpsc::unbounded_channel;
use tracing_subscriber::EnvFilter;

/// The Unix group whose members may expose an image (dev-version policy).
const BOOTDRIVE_GROUP: &str = "bootdrive";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("bootdrived=info")),
        )
        .init();

    tracing::info!("bootdrived starting");

    let authorizer: Arc<dyn Authorizer> = if std::env::var_os("BOOTDRIVE_ALLOW_ALL").is_some() {
        tracing::warn!("BOOTDRIVE_ALLOW_ALL set: authorization is disabled (development only)");
        Arc::new(AllowAllAuthorizer)
    } else {
        Arc::new(GroupAuthorizer::for_group(BOOTDRIVE_GROUP))
    };

    let backend = Box::new(UsbGadgetBackend::new());
    let signaller = usb_signaller::detect();
    let recovery = RecoveryStore::default();

    let (tx, rx) = unbounded_channel();
    let mut manager = Manager::new(backend, signaller, recovery);
    manager.set_event_sink(tx);

    // Reconcile any leftover state from an unclean previous run.
    manager.recover();

    let manager = Arc::new(Mutex::new(manager));
    let service = BootDriveService::new(manager, authorizer);

    let connection = zbus::connection::Builder::system()?
        .name(DBUS_SERVICE_NAME)?
        .serve_at(DBUS_OBJECT_PATH, service)?
        .build()
        .await?;

    tracing::info!("owning {DBUS_SERVICE_NAME} on the system bus");

    tokio::spawn(run_event_forwarder(connection.clone(), rx));

    wait_for_shutdown().await;
    tracing::info!("bootdrived shutting down");
    Ok(())
}

/// Block until the process receives SIGINT or SIGTERM.
async fn wait_for_shutdown() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    let mut int = signal(SignalKind::interrupt()).expect("install SIGINT handler");
    tokio::select! {
        _ = term.recv() => {},
        _ = int.recv() => {},
    }
}
