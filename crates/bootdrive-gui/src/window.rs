//! The single BootDrive window.
//!
//! Built programmatically with libadwaita widgets (rather than a `.ui`
//! template) so the whole surface is self-contained and easy to reason about
//! for a one-screen mobile app. It reflects the daemon's state and drives it
//! through [`DaemonClient`].

use std::cell::RefCell;
use std::rc::Rc;

use bootdrive_common::{DriveState, ImageMode, StateInfo};
use gtk4 as gtk;
use libadwaita as adw;

use adw::prelude::*;

use crate::daemon_proxy::{headline, DaemonClient, DaemonCommand, DaemonUpdate, UnreachableReason};
use crate::image_selection::{human_size, in_flatpak, resolve_host_path, Selection};

/// Everything the callbacks need to share.
struct Ui {
    window: adw::ApplicationWindow,
    toasts: adw::ToastOverlay,
    stack: gtk::Stack,

    // Main page widgets.
    status_title: gtk::Label,
    status_detail: gtk::Label,
    image_row: adw::ActionRow,
    mode_row: adw::ComboRow,
    change_button: gtk::Button,
    primary_button: gtk::Button,

    // Setup page.
    setup_status: adw::StatusPage,

    client: DaemonClient,
    selection: RefCell<Option<Selection>>,
    state: RefCell<StateInfo>,
    // Whether a call is in flight (disables inputs).
    busy: RefCell<bool>,
}

/// Build and present the main window for `app`.
pub fn build(app: &adw::Application) -> adw::ApplicationWindow {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("BootDrive")
        .default_width(400)
        .default_height(520)
        .width_request(300)
        .build();

    let toasts = adw::ToastOverlay::new();
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar.add_top_bar(&header);

    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .vexpand(true)
        .build();

    // --- Main page ---------------------------------------------------------
    let page = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(12)
        .margin_end(12)
        .build();

    // USB status group.
    let status_group = adw::PreferencesGroup::builder().title("USB status").build();
    let status_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    let status_title = gtk::Label::builder()
        .halign(gtk::Align::Start)
        .css_classes(vec!["title-2".to_string()])
        .label("Connecting…")
        .build();
    let status_detail = gtk::Label::builder()
        .halign(gtk::Align::Start)
        .wrap(true)
        .css_classes(vec!["dim-label".to_string()])
        .label("")
        .build();
    status_box.append(&status_title);
    status_box.append(&status_detail);
    status_group.add(&status_box);
    page.append(&status_group);

    // Selected-image group.
    let image_group = adw::PreferencesGroup::builder()
        .title("Selected image")
        .build();
    let image_row = adw::ActionRow::builder()
        .title("No image selected")
        .subtitle("Choose an .iso, .img or .raw file")
        .build();
    let change_button = gtk::Button::builder()
        .label("Change")
        .valign(gtk::Align::Center)
        .build();
    image_row.add_suffix(&change_button);
    image_group.add(&image_row);

    let mode_model = gtk::StringList::new(&["USB CD-ROM", "USB disk"]);
    let mode_row = adw::ComboRow::builder()
        .title("Exposure mode")
        .subtitle("How the host sees the image")
        .model(&mode_model)
        .build();
    mode_row.set_visible(false);
    image_group.add(&mode_row);
    page.append(&image_group);

    // Primary action button.
    let primary_button = gtk::Button::builder()
        .label("Expose over USB")
        .css_classes(vec!["suggested-action".to_string(), "pill".to_string()])
        .halign(gtk::Align::Fill)
        .margin_top(6)
        .sensitive(false)
        .build();
    page.append(&primary_button);

    stack.add_named(&page, Some("main"));

    // --- Setup / unavailable page -----------------------------------------
    let setup_status = adw::StatusPage::builder()
        .icon_name("drive-harddisk-usb-symbolic")
        .title("Helper not available")
        .description("The BootDrive system helper is not installed or not running.")
        .build();
    stack.add_named(&setup_status, Some("setup"));

    toolbar.set_content(Some(&stack));
    toasts.set_child(Some(&toolbar));
    window.set_content(Some(&toasts));

    let ui = Rc::new(Ui {
        window: window.clone(),
        toasts,
        stack,
        status_title,
        status_detail,
        image_row,
        mode_row,
        change_button,
        primary_button,
        setup_status,
        client: DaemonClient::spawn(),
        selection: RefCell::new(None),
        state: RefCell::new(StateInfo::default()),
        busy: RefCell::new(false),
    });

    wire_callbacks(&ui);
    listen_for_updates(&ui);
    ui.refresh();

    window
}

fn wire_callbacks(ui: &Rc<Ui>) {
    // Change / choose image.
    {
        let ui = ui.clone();
        let button = ui.change_button.clone();
        button.connect_clicked(move |_| choose_image(&ui));
    }

    // Mode selection.
    {
        let ui = ui.clone();
        let row = ui.mode_row.clone();
        row.connect_selected_notify(move |row| {
            if let Some(sel) = ui.selection.borrow_mut().as_mut() {
                sel.default_mode = if row.selected() == 0 {
                    ImageMode::Cdrom
                } else {
                    ImageMode::Disk
                };
            }
        });
    }

    // Primary action: Expose or Eject depending on state.
    {
        let ui = ui.clone();
        let button = ui.primary_button.clone();
        button.connect_clicked(move |_| {
            let state = ui.state.borrow().state;
            match state {
                DriveState::Active => confirm_and_eject(&ui),
                DriveState::Idle | DriveState::Error => activate(&ui),
                _ => {}
            }
        });
    }
}

/// Open the portal-aware file chooser.
fn choose_image(ui: &Rc<Ui>) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Disk images"));
    for pat in ["*.iso", "*.img", "*.raw"] {
        filter.add_pattern(pat);
    }
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);

    let dialog = gtk::FileDialog::builder()
        .title("Select a disk image")
        .filters(&filters)
        .modal(true)
        .build();

    let parent = ui.window.clone();
    let ui = ui.clone();
    dialog.open(
        Some(&parent),
        gio::Cancellable::NONE,
        move |result| match result {
            Ok(file) => {
                if let Some(path) = file.path() {
                    let host = resolve_host_path(&path);
                    let size = std::fs::metadata(&host).ok().map(|m| m.len());
                    let selection = Selection::from_host_path(host, size);
                    *ui.selection.borrow_mut() = Some(selection);
                    ui.refresh();
                } else {
                    ui.toast("That file could not be resolved to a readable path.");
                }
            }
            Err(e) => {
                // Cancellation is not an error worth surfacing.
                if !e.matches(gtk::DialogError::Dismissed) {
                    ui.toast("The file could not be opened.");
                }
            }
        },
    );
}

/// Ask the daemon to expose the current selection.
fn activate(ui: &Rc<Ui>) {
    let Some(selection) = ui.selection.borrow().clone() else {
        ui.toast("Select an image first.");
        return;
    };
    ui.set_busy(true);
    let cmd = DaemonCommand::Activate {
        path: selection.host_path.to_string_lossy().into_owned(),
        display_name: selection.display_name.clone(),
        mode: selection.default_mode,
    };
    send(ui, cmd);
}

/// Confirm before ejecting an actively-exposed image, then eject.
fn confirm_and_eject(ui: &Rc<Ui>) {
    let dialog = adw::AlertDialog::builder()
        .heading("Eject image?")
        .body("The connected computer will lose access to this drive.")
        .build();
    dialog.add_responses(&[("cancel", "Cancel"), ("eject", "Eject")]);
    dialog.set_response_appearance("eject", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    let parent = ui.window.clone();
    let ui = ui.clone();
    dialog.choose(&parent, gio::Cancellable::NONE, move |resp| {
        if resp == "eject" {
            ui.set_busy(true);
            send(&ui, DaemonCommand::Deactivate);
        }
    });
}

/// Send a command to the D-Bus worker.
fn send(ui: &Rc<Ui>, cmd: DaemonCommand) {
    let tx = ui.client.commands.clone();
    glib::spawn_future_local(async move {
        let _ = tx.send(cmd).await;
    });
}

/// Drain updates from the D-Bus worker on the GTK main loop.
fn listen_for_updates(ui: &Rc<Ui>) {
    let ui = ui.clone();
    let rx = ui.client.updates.clone();
    glib::spawn_future_local(async move {
        while let Ok(update) = rx.recv().await {
            match update {
                DaemonUpdate::State(info) => {
                    *ui.state.borrow_mut() = info;
                    ui.set_busy(false);
                    ui.refresh();
                }
                DaemonUpdate::Error { code, message } => {
                    ui.set_busy(false);
                    ui.toast(&format!("{message} ({code})"));
                }
                DaemonUpdate::Unreachable(reason) => {
                    ui.set_busy(false);
                    ui.show_setup(&reason);
                }
            }
        }
    });
}

impl Ui {
    fn toast(&self, text: &str) {
        self.toasts.add_toast(adw::Toast::new(text));
    }

    fn set_busy(&self, busy: bool) {
        *self.busy.borrow_mut() = busy;
        self.refresh_controls();
    }

    fn show_setup(&self, reason: &UnreachableReason) {
        let (title, desc) = match reason {
            UnreachableReason::NotRunning => (
                "Helper not running",
                "The BootDrive system helper (bootdrived) is not installed or not \
                 started. Install the postmarketOS package and start the service.",
            ),
            UnreachableReason::PermissionDenied => (
                "Permission needed",
                "You are not allowed to talk to the BootDrive helper. Add your user \
                 to the 'bootdrive' group and try again.",
            ),
            UnreachableReason::Other(_) => (
                "Helper unavailable",
                "BootDrive could not reach its system helper over D-Bus.",
            ),
        };
        self.setup_status.set_title(title);
        self.setup_status.set_description(Some(desc));
        self.stack.set_visible_child_name("setup");
    }

    /// Recompute every label and control from the current state + selection.
    fn refresh(&self) {
        let info = self.state.borrow().clone();

        if info.state == DriveState::Unavailable && !in_flatpak() {
            // On a dev box with no daemon this is normal; still show setup.
        }

        self.stack.set_visible_child_name("main");
        self.status_title.set_text(headline(info.state));
        self.status_detail.set_text(&status_detail_text(&info));

        // Selected image display.
        if info.state == DriveState::Active {
            self.image_row.set_title(&info.display_name);
            self.image_row
                .set_subtitle(&format!("{} · Read-only", info.mode.label()));
            self.mode_row.set_visible(false);
            self.change_button.set_visible(false);
        } else if let Some(sel) = self.selection.borrow().as_ref() {
            self.image_row.set_title(&sel.display_name);
            let size = sel
                .size
                .map(human_size)
                .unwrap_or_else(|| "unknown size".to_string());
            self.image_row
                .set_subtitle(&format!("{} · {}", size, sel.default_mode.label()));
            self.change_button.set_visible(true);
            self.mode_row.set_visible(sel.hybrid);
            if sel.hybrid {
                self.mode_row
                    .set_selected(if sel.default_mode == ImageMode::Cdrom {
                        0
                    } else {
                        1
                    });
            }
        } else {
            self.image_row.set_title("No image selected");
            self.image_row
                .set_subtitle("Choose an .iso, .img or .raw file");
            self.change_button.set_visible(true);
            self.mode_row.set_visible(false);
        }

        self.refresh_controls();
    }

    /// Update just the interactive controls (button label + sensitivity).
    fn refresh_controls(&self) {
        let info = self.state.borrow().clone();
        let busy = *self.busy.borrow();
        let has_selection = self.selection.borrow().is_some();

        let transitioning = matches!(info.state, DriveState::Preparing | DriveState::Ejecting);
        let inputs_enabled = !busy && !transitioning;

        // Plan: disable selection while preparing/ejecting.
        self.change_button.set_sensitive(inputs_enabled);
        self.mode_row.set_sensitive(inputs_enabled);

        match info.state {
            DriveState::Active => {
                self.primary_button.set_label("Eject");
                self.primary_button
                    .set_css_classes(&["destructive-action", "pill"]);
                self.primary_button.set_sensitive(inputs_enabled);
            }
            DriveState::Idle | DriveState::Error => {
                self.primary_button.set_label("Expose over USB");
                self.primary_button
                    .set_css_classes(&["suggested-action", "pill"]);
                self.primary_button
                    .set_sensitive(inputs_enabled && has_selection);
            }
            DriveState::Preparing => {
                self.primary_button.set_label("Preparing…");
                self.primary_button.set_sensitive(false);
            }
            DriveState::Ejecting => {
                self.primary_button.set_label("Ejecting…");
                self.primary_button.set_sensitive(false);
            }
            DriveState::Unavailable => {
                self.primary_button.set_label("Expose over USB");
                self.primary_button.set_sensitive(false);
            }
        }
    }
}

/// The detail line under the big status headline.
fn status_detail_text(info: &StateInfo) -> String {
    match info.state {
        DriveState::Unavailable => {
            if info.last_error.is_empty() {
                "BootDrive is not available on this device.".to_string()
            } else {
                info.last_error.clone()
            }
        }
        DriveState::Idle => "Select an image and expose it over USB.".to_string(),
        DriveState::Preparing => "Setting up the USB gadget…".to_string(),
        DriveState::Active => match info.mode {
            ImageMode::Cdrom => "Connected as a bootable CD-ROM.".to_string(),
            ImageMode::Disk => "Connected as a bootable USB disk.".to_string(),
        },
        DriveState::Ejecting => "Returning to normal USB behaviour…".to_string(),
        DriveState::Error => {
            if info.last_error.is_empty() {
                "Something went wrong.".to_string()
            } else {
                info.last_error.clone()
            }
        }
    }
}
