//! `probe` — the low-level proof-of-concept from plan section 23.
//!
//! Run on the target device to validate the whole handoff before the D-Bus
//! service exists:
//!
//! ```sh
//! sudo cargo run --package bootdrived --bin probe -- /absolute/path/to/image.iso
//! ```
//!
//! It confirms a usable UDC and mass-storage support, detects a bound `g1`,
//! releases `usb-signaller`, creates and binds the `bootdrive` gadget for the
//! supplied ISO (read-only CD-ROM), waits for Ctrl-C, then cleans up and
//! restores `usb-signaller`.

use std::path::PathBuf;
use std::process::{Command, ExitCode};

use bootdrive_common::ImageMode;
use bootdrived::image::{self, Caller};
use bootdrived::usb::{UsbBackend, UsbGadgetBackend, GADGET_NAME};
use bootdrived::usb_signaller;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter("probe=info,bootdrived=info")
        .init();

    let Some(arg) = std::env::args().nth(1) else {
        eprintln!("usage: probe /absolute/path/to/image.iso");
        return ExitCode::FAILURE;
    };

    match run(PathBuf::from(arg)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("probe failed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    if !nix_is_root() {
        return Err("must be run as root (try sudo)".into());
    }

    // (2) Ensure the required kernel modules can load.
    ensure_module("libcomposite");
    ensure_module("usb_f_mass_storage");

    let mut backend = UsbGadgetBackend::new();

    // (1) Confirm exactly one usable UDC exists.
    let caps = backend.probe()?;
    println!("UDC: {}", caps.udc);

    // Validate the image as the root caller.
    let caller = Caller {
        uid: 0,
        gids: vec![0],
    };
    let image = image::validate(&path, "", &caller)?;
    println!(
        "image: {} ({:.1} MiB)",
        image.display_name,
        image.size as f64 / (1024.0 * 1024.0)
    );

    let signaller = usb_signaller::detect();

    // (3) Detect whether usb-signaller / g1 is currently active.
    let was_running = signaller.is_running().unwrap_or(false);
    println!("usb-signaller running: {was_running}");

    // (4) Release usb-signaller and free the UDC.
    if was_running {
        println!("stopping usb-signaller…");
        signaller.stop()?;
    }
    backend.release_foreign_gadgets()?;
    backend.remove_stale_gadget()?;

    // (5,6) Create and bind the bootdrive gadget as a read-only CD-ROM.
    println!("creating '{GADGET_NAME}' gadget…");
    if let Err(e) = backend.activate(&image, ImageMode::Cdrom) {
        // Roll back on failure.
        let _ = backend.deactivate();
        if was_running {
            let _ = signaller.start();
        }
        return Err(Box::new(e));
    }
    println!("active: the host should now see a read-only USB CD-ROM.");
    println!("press Ctrl-C to eject and restore normal USB behaviour.");

    // (7) Wait for Ctrl-C.
    wait_for_ctrl_c();

    // (8) Remove the bootdrive gadget.
    println!("\nejecting…");
    backend.deactivate()?;

    // (9) Restore usb-signaller.
    if was_running {
        println!("restarting usb-signaller…");
        signaller.start()?;
    }
    println!("done.");
    Ok(())
}

fn nix_is_root() -> bool {
    // Avoid an extra crate: read the effective uid via `id -u` fallback to geteuid.
    rustix::process::geteuid().is_root()
}

fn ensure_module(name: &str) {
    match Command::new("/sbin/modprobe").arg(name).status() {
        Ok(s) if s.success() => println!("module {name}: ok"),
        Ok(_) => eprintln!("warning: modprobe {name} returned non-zero (may be built-in)"),
        Err(e) => eprintln!("warning: could not run modprobe {name}: {e}"),
    }
}

fn wait_for_ctrl_c() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    if let Err(e) = ctrlc::set_handler(move || s.store(true, Ordering::SeqCst)) {
        eprintln!("warning: could not install Ctrl-C handler: {e}");
    }
    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
