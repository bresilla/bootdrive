//! The libadwaita application object.

use libadwaita as adw;

use adw::prelude::*;

use crate::window;

/// The application ID; also the Flatpak app id.
pub const APP_ID: &str = "net.bresilla.BootDrive";

/// Construct the [`adw::Application`], wiring up window creation on activate.
pub fn build() -> adw::Application {
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::default())
        .build();

    app.connect_activate(|app| {
        // Reuse the existing window if the app is re-activated.
        if let Some(win) = app.active_window() {
            win.present();
            return;
        }
        let win = window::build(app);
        win.present();
    });

    // Ctrl+Q quits.
    app.set_accels_for_action("window.close", &["<Primary>q"]);

    app
}
