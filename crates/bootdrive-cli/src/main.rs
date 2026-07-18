//! `bootdrive` — the command-line frontend.
//!
//! A thin native client of the `net.bresilla.BootDrive1` system-D-Bus service.
//! Handy for headless/SSH use on the phone and for scripting. It holds no
//! privileges itself; the backend does all the work.

use anyhow::{Context, Result};
use bootdrive_common::{DriveState, ImageMode, StateInfo};
use clap::{Parser, Subcommand};
use zbus::proxy;

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

#[derive(Parser)]
#[command(
    name = "bootdrive",
    about = "Expose a disk image as a bootable USB drive",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the current USB state.
    Status,
    /// Expose an image over USB.
    Expose {
        /// Path to the .iso/.img/.raw image.
        image: std::path::PathBuf,
        /// Force USB CD-ROM mode.
        #[arg(long, conflicts_with = "disk")]
        cdrom: bool,
        /// Force USB disk mode.
        #[arg(long)]
        disk: bool,
        /// Display name (defaults to the file name).
        #[arg(long)]
        name: Option<String>,
    },
    /// Eject the current image.
    Eject,
    /// Follow state and error events until interrupted.
    Watch,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let cli = Cli::parse();
    let conn = zbus::Connection::system()
        .await
        .context("cannot connect to the system bus")?;
    let proxy = BootDriveProxy::new(&conn)
        .await
        .context("bootdrived is not reachable (is the service installed and running?)")?;

    match cli.command {
        Command::Status => {
            let state = proxy.get_state().await.context("could not read state")?;
            print_state(&state);
        }
        Command::Expose {
            image,
            cdrom,
            disk,
            name,
        } => {
            let path = std::fs::canonicalize(&image)
                .with_context(|| format!("no such file: {}", image.display()))?;
            let display_name = name.unwrap_or_else(|| {
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "image".to_string())
            });
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
            proxy
                .activate(&path.to_string_lossy(), &display_name, mode.as_wire())
                .await
                .map_err(explain)?;
            println!("Exposing {display_name} as {}.", mode.label());
        }
        Command::Eject => {
            proxy.deactivate().await.map_err(explain)?;
            println!("Ejected.");
        }
        Command::Watch => watch(&proxy).await?,
    }

    Ok(())
}

/// Turn a coded method error ("code: message") into a readable message.
fn explain(err: zbus::Error) -> anyhow::Error {
    if let zbus::Error::MethodError(_, Some(text), _) = &err {
        anyhow::anyhow!("{text}")
    } else {
        anyhow::anyhow!("{err}")
    }
}

fn print_state(state: &StateInfo) {
    let headline = match state.state {
        DriveState::Unavailable => "unavailable",
        DriveState::Idle => "ready",
        DriveState::Preparing => "preparing",
        DriveState::Active => "active",
        DriveState::Ejecting => "ejecting",
        DriveState::Error => "error",
    };
    println!("State:  {headline}");
    if !state.display_name.is_empty() {
        println!("Image:  {} ({})", state.display_name, state.mode.label());
    }
    if !state.udc.is_empty() {
        println!("UDC:    {}", state.udc);
    }
    if !state.last_error.is_empty() {
        println!("Error:  {}", state.last_error);
    }
}

async fn watch(proxy: &BootDriveProxy<'_>) -> Result<()> {
    use futures_util::StreamExt;

    if let Ok(state) = proxy.get_state().await {
        print_state(&state);
        println!("---");
    }

    let mut states = proxy.receive_state_changed().await?;
    let mut errors = proxy.receive_error_occurred().await?;
    println!("Watching for changes (Ctrl-C to stop)…");
    loop {
        tokio::select! {
            Some(sig) = states.next() => {
                if let Ok(args) = sig.args() {
                    print_state(&args.info);
                    println!("---");
                }
            }
            Some(sig) = errors.next() => {
                if let Ok(args) = sig.args() {
                    eprintln!("error: {} ({})", args.message, args.code);
                }
            }
            else => break,
        }
    }
    Ok(())
}
