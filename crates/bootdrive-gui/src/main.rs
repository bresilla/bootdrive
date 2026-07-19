//! BootDrive GUI entry point.
//!
//! A thin GTK4/libadwaita front-end that selects one disk image and drives the
//! privileged `bootdrived` helper over the system bus. It holds no privileges
//! itself and does nothing to configfs.

mod application;
mod catalog;
mod library;
mod usb_moded;
mod window;

use libadwaita as adw;

use adw::prelude::*;
use tracing_subscriber::EnvFilter;

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("bootdrive_gui=info")),
        )
        .init();

    // libadwaita must be initialised before any adw widgets are constructed.
    adw::init().expect("failed to initialise libadwaita");

    let app = application::build();
    app.run()
}
