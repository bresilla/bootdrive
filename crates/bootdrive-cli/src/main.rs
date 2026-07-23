//! The `bootdrive` command-line frontend.
//!
//! Drives postmarketOS's `usb-signaller` directly over `com.meego.usb_moded`.
//! No BootDrive daemon involved.

use anyhow::{bail, Context, Result};
use bootdrive_common::{point_current_image, ImageMode, MODE_NORMAL};
use clap::{Parser, Subcommand};
use zbus::proxy;

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

    #[zbus(signal, name = "sig_usb_event_ind")]
    fn sig_usb_event_ind(&self, event: String) -> zbus::Result<()>;
}

#[derive(Parser)]
#[command(
    name = "bootdrive",
    about = "Expose a disk image as a bootable USB drive (via usb-signaller)",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the current USB mode.
    Status,
    /// Expose an image over USB as mass storage.
    Expose {
        /// Path to the .iso/.img/.raw image.
        image: std::path::PathBuf,
        /// Force USB CD-ROM mode.
        #[arg(long, conflicts_with = "disk")]
        cdrom: bool,
        /// Force USB disk mode.
        #[arg(long)]
        disk: bool,
    },
    /// Eject (return to normal USB mode).
    Eject,
    /// Follow USB mode changes until interrupted.
    Watch,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let conn = zbus::Connection::system()
        .await
        .context("cannot connect to the system bus")?;
    // usb-signaller blocks org.freedesktop.DBus.Properties in its policy, so
    // disable zbus property caching (it would call GetAll and be denied).
    let proxy = UsbModedProxy::builder(&conn)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .context("usb-signaller (com.meego.usb_moded) is not reachable")?;

    match cli.command {
        Command::Status => {
            let mode = proxy.mode_request().await.context("mode_request failed")?;
            println!("Mode: {mode}");
        }
        Command::Expose { image, cdrom, disk } => {
            let path = std::fs::canonicalize(&image)
                .with_context(|| format!("no such file: {}", image.display()))?;
            let mode = if cdrom {
                ImageMode::Cdrom
            } else if disk {
                ImageMode::Disk
            } else {
                let ext = path
                    .extension()
                    .map(|e| e.to_string_lossy().into_owned())
                    .unwrap_or_default();
                ImageMode::default_for_extension(&ext)
            };
            // usb-signaller reads the image from its configured storage_path,
            // which points at the current-image symlink; aim it at this file.
            point_current_image(&path).context("could not update the current image link")?;
            let target_mode = mode.mode_str();
            let result = proxy
                .set_mode(target_mode)
                .await
                .context("set_mode failed")?;
            if result == target_mode {
                println!("Exposing {} as {}.", path.display(), mode.label());
            } else {
                bail!("usb-signaller stayed in '{result}', check its logs");
            }
        }
        Command::Eject => {
            proxy
                .set_mode(MODE_NORMAL)
                .await
                .context("set_mode failed")?;
            println!("Ejected (back to {MODE_NORMAL}).");
        }
        Command::Watch => watch(&proxy).await?,
    }

    Ok(())
}

async fn watch(proxy: &UsbModedProxy<'_>) -> Result<()> {
    use futures_util::StreamExt;
    if let Ok(mode) = proxy.mode_request().await {
        println!("Mode: {mode}");
    }
    let mut events = proxy.receive_sig_usb_event_ind().await?;
    println!("Watching USB mode changes (Ctrl-C to stop)…");
    while let Some(sig) = events.next().await {
        if let Ok(args) = sig.args() {
            println!("event: {}", args.event);
        }
    }
    Ok(())
}
