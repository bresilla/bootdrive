//! The BootDrive window: an adaptive sidebar (image library) + a content pane
//! (selected image + expose/eject), with a primary menu and About.
//!
//! Built with `AdwNavigationSplitView`, which collapses to a single pane on the
//! phone. State comes from usb-signaller via [`DaemonClient`].

use std::cell::RefCell;
use std::rc::Rc;

use bootdrive_common::DriveState;
use gtk4 as gtk;
use libadwaita as adw;

use adw::prelude::*;

use crate::library::{human_size, ImageEntry, Library};
use crate::usb_moded::{DaemonClient, DaemonCommand, DaemonUpdate, UnreachableReason};

/// Shared UI state.
struct Ui {
    window: adw::ApplicationWindow,
    split: adw::NavigationSplitView,
    toasts: adw::ToastOverlay,

    // Sidebar.
    status_row: adw::ActionRow,
    list: gtk::ListBox,

    // Content.
    content_stack: gtk::Stack,
    content_title: adw::WindowTitle,
    image_group: adw::PreferencesGroup,
    size_row: adw::ActionRow,
    mode_row: adw::ComboRow,
    path_row: adw::ActionRow,
    primary_button: gtk::Button,
    setup_status: adw::StatusPage,

    client: DaemonClient,
    library: RefCell<Library>,
    selected: RefCell<Option<usize>>,
    state: RefCell<DriveState>,
    busy: RefCell<bool>,
    available: RefCell<bool>,
}

/// Build and present the main window.
pub fn build(app: &adw::Application) -> adw::ApplicationWindow {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("BootDrive")
        .default_width(820)
        .default_height(560)
        .width_request(320)
        .height_request(320)
        .build();

    let toasts = adw::ToastOverlay::new();

    // ---- Sidebar ----------------------------------------------------------
    let add_button = gtk::Button::from_icon_name("list-add-symbolic");
    add_button.set_tooltip_text(Some("Add an image"));

    let menu = gio::Menu::new();
    menu.append(Some("About BootDrive"), Some("win.about"));
    let menu_button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text("Main menu")
        .menu_model(&menu)
        .build();

    let sidebar_header = adw::HeaderBar::new();
    sidebar_header.pack_start(&add_button);
    sidebar_header.pack_end(&menu_button);

    let status_row = adw::ActionRow::builder()
        .title("Connecting…")
        .subtitle("USB status")
        .build();
    let status_icon = gtk::Image::from_icon_name("content-loading-symbolic");
    status_row.add_prefix(&status_icon);
    let status_group = adw::PreferencesGroup::new();
    status_group.add(&status_row);

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .css_classes(vec!["boxed-list".to_string()])
        .build();
    let images_group = adw::PreferencesGroup::builder().title("Images").build();
    images_group.add(&list);

    let sidebar_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    sidebar_box.append(&status_group);
    sidebar_box.append(&images_group);
    let sidebar_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&sidebar_box)
        .build();

    let sidebar_toolbar = adw::ToolbarView::new();
    sidebar_toolbar.add_top_bar(&sidebar_header);
    sidebar_toolbar.set_content(Some(&sidebar_scroll));
    let sidebar_page = adw::NavigationPage::new(&sidebar_toolbar, "BootDrive");

    // ---- Content ----------------------------------------------------------
    let content_title = adw::WindowTitle::new("BootDrive", "");
    let content_header = adw::HeaderBar::new();
    content_header.set_title_widget(Some(&content_title));

    // Detail page.
    let image_group = adw::PreferencesGroup::builder()
        .title("Selected image")
        .build();
    let size_row = adw::ActionRow::builder().title("Size").build();
    let mode_model = gtk::StringList::new(&["USB CD-ROM", "USB disk"]);
    let mode_row = adw::ComboRow::builder()
        .title("Exposure mode")
        .subtitle("How the computer sees it")
        .model(&mode_model)
        .build();
    let path_row = adw::ActionRow::builder().title("Location").build();
    path_row.add_css_class("property");
    image_group.add(&size_row);
    image_group.add(&mode_row);
    image_group.add(&path_row);

    let primary_button = gtk::Button::builder()
        .label("Expose over USB")
        .css_classes(vec!["suggested-action".to_string(), "pill".to_string()])
        .halign(gtk::Align::Center)
        .margin_top(6)
        .build();
    let button_clamp = adw::Clamp::builder()
        .maximum_size(360)
        .child(&primary_button)
        .build();

    let detail_page = adw::PreferencesPage::new();
    detail_page.add(&image_group);
    let button_group = adw::PreferencesGroup::new();
    button_group.add(&button_clamp);
    detail_page.add(&button_group);

    // Empty + setup pages.
    let empty_status = adw::StatusPage::builder()
        .icon_name("drive-removable-media-symbolic")
        .title("No image selected")
        .description("Pick an image from the sidebar, or add a new .iso/.img/.raw.")
        .build();
    let setup_status = adw::StatusPage::builder()
        .icon_name("dialog-warning-symbolic")
        .title("usb-signaller unavailable")
        .build();

    let content_stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .build();
    content_stack.add_named(&empty_status, Some("empty"));
    content_stack.add_named(&detail_page, Some("detail"));
    content_stack.add_named(&setup_status, Some("setup"));

    let content_toolbar = adw::ToolbarView::new();
    content_toolbar.add_top_bar(&content_header);
    content_toolbar.set_content(Some(&content_stack));
    let content_page = adw::NavigationPage::new(&content_toolbar, "Image");

    // ---- Split view + adaptive breakpoint ---------------------------------
    let split = adw::NavigationSplitView::builder()
        .sidebar(&sidebar_page)
        .content(&content_page)
        .min_sidebar_width(280.0)
        .build();
    toasts.set_child(Some(&split));
    window.set_content(Some(&toasts));

    if let Ok(cond) = adw::BreakpointCondition::parse("max-width: 550sp") {
        let bp = adw::Breakpoint::new(cond);
        bp.add_setter(&split, "collapsed", Some(&true.to_value()));
        window.add_breakpoint(bp);
    }

    let ui = Rc::new(Ui {
        window: window.clone(),
        split,
        toasts,
        status_row,
        list,
        content_stack,
        content_title,
        image_group,
        size_row,
        mode_row,
        path_row,
        primary_button,
        setup_status,
        client: DaemonClient::spawn(),
        library: RefCell::new(Library::load()),
        selected: RefCell::new(None),
        state: RefCell::new(DriveState::Unavailable),
        busy: RefCell::new(false),
        available: RefCell::new(false),
    });

    install_about_action(&ui);
    wire(&ui, &add_button);
    rebuild_list(&ui);
    listen_for_updates(&ui);
    ui.refresh();

    window
}

fn install_about_action(ui: &Rc<Ui>) {
    let action = gio::SimpleAction::new("about", None);
    let window = ui.window.clone();
    action.connect_activate(move |_, _| {
        let about = adw::AboutWindow::builder()
            .application_name("BootDrive")
            .application_icon("net.bresilla.BootDrive")
            .developer_name("Kushtrim Bresilla")
            .version("0.1.0")
            .comments("Expose a disk image as a bootable USB drive.")
            .website("https://github.com/bresilla/bootdrive")
            .issue_url("https://github.com/bresilla/bootdrive/issues")
            .license_type(gtk::License::MitX11)
            .build();
        about.set_transient_for(Some(&window));
        about.set_modal(true);
        about.present();
    });
    ui.window.add_action(&action);
}

fn wire(ui: &Rc<Ui>, add_button: &gtk::Button) {
    {
        let ui = ui.clone();
        add_button.connect_clicked(move |_| choose_image(&ui));
    }
    {
        let ui = ui.clone();
        let list = ui.list.clone();
        list.connect_row_activated(move |_, row| {
            select(&ui, row.index() as usize);
        });
    }
    {
        let ui = ui.clone();
        let row = ui.mode_row.clone();
        row.connect_selected_notify(move |row| {
            if let Some(i) = *ui.selected.borrow() {
                let mut lib = ui.library.borrow_mut();
                if let Some(e) = lib.entries.get_mut(i) {
                    e.cdrom = row.selected() == 0;
                    lib.save();
                }
            }
        });
    }
    {
        let ui = ui.clone();
        let button = ui.primary_button.clone();
        button.connect_clicked(move |_| {
            let state = *ui.state.borrow();
            match state {
                DriveState::Active => confirm_and_eject(&ui),
                DriveState::Idle | DriveState::Error => activate(&ui),
                _ => {}
            }
        });
    }
}

/// Rebuild the sidebar list from the library.
fn rebuild_list(ui: &Rc<Ui>) {
    while let Some(child) = ui.list.first_child() {
        ui.list.remove(&child);
    }
    let lib = ui.library.borrow();
    for entry in &lib.entries {
        let row = adw::ActionRow::builder()
            .title(&entry.display_name)
            .subtitle(subtitle_for(entry))
            .activatable(true)
            .build();
        let icon = gtk::Image::from_icon_name(if entry.cdrom {
            "media-optical-symbolic"
        } else {
            "drive-harddisk-symbolic"
        });
        row.add_prefix(&icon);
        if !Library::exists(entry) {
            row.add_css_class("dim-label");
            row.set_subtitle("missing — file not found");
        }

        let remove_btn = gtk::Button::from_icon_name("user-trash-symbolic");
        remove_btn.set_valign(gtk::Align::Center);
        remove_btn.add_css_class("flat");
        remove_btn.set_tooltip_text(Some("Remove from library"));
        row.add_suffix(&remove_btn);
        {
            let ui = ui.clone();
            let row_weak = row.downgrade();
            remove_btn.connect_clicked(move |_| {
                if let Some(row) = row_weak.upgrade() {
                    remove_image(&ui, row.index() as usize);
                }
            });
        }

        ui.list.append(&row);
    }
}

/// Remove the library entry at `index` and fix up the selection.
fn remove_image(ui: &Rc<Ui>, index: usize) {
    ui.library.borrow_mut().remove(index);
    {
        let mut sel = ui.selected.borrow_mut();
        match *sel {
            Some(i) if i == index => *sel = None,
            Some(i) if i > index => *sel = Some(i - 1),
            _ => {}
        }
    }
    rebuild_list(ui);
    ui.refresh();
}

fn subtitle_for(entry: &ImageEntry) -> String {
    let size = entry
        .size
        .map(human_size)
        .unwrap_or_else(|| "unknown size".to_string());
    format!("{} · {}", size, entry.mode().label())
}

/// Select the library entry at `index` and show the content pane.
fn select(ui: &Rc<Ui>, index: usize) {
    if index >= ui.library.borrow().entries.len() {
        return;
    }
    *ui.selected.borrow_mut() = Some(index);
    ui.split.set_show_content(true);
    ui.refresh();
}

/// Open the file chooser, add the result to the library, and select it.
fn choose_image(ui: &Rc<Ui>) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Disk images"));
    for pat in ["*.iso", "*.img", "*.raw"] {
        filter.add_pattern(pat);
    }
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);

    let dialog = gtk::FileDialog::builder()
        .title("Add a disk image")
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
                    let size = std::fs::metadata(&path).ok().map(|m| m.len());
                    let entry = ImageEntry::from_path(path, size);
                    ui.library.borrow_mut().add(entry);
                    rebuild_list(&ui);
                    let last = ui.library.borrow().entries.len().saturating_sub(1);
                    select(&ui, last);
                } else {
                    ui.toast("That file could not be resolved to a readable path.");
                }
            }
            Err(e) => {
                if !e.matches(gtk::DialogError::Dismissed) {
                    ui.toast("The file could not be opened.");
                }
            }
        },
    );
}

fn activate(ui: &Rc<Ui>) {
    let Some(i) = *ui.selected.borrow() else {
        ui.toast("Select an image first.");
        return;
    };
    let entry = ui.library.borrow().entries.get(i).cloned();
    let Some(entry) = entry else { return };
    if !Library::exists(&entry) {
        ui.toast("That file no longer exists.");
        return;
    }
    ui.set_busy(true);
    send(
        ui,
        DaemonCommand::Activate {
            path: entry.path.clone(),
            mode: entry.mode(),
        },
    );
}

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

fn send(ui: &Rc<Ui>, cmd: DaemonCommand) {
    let tx = ui.client.commands.clone();
    glib::spawn_future_local(async move {
        let _ = tx.send(cmd).await;
    });
}

fn listen_for_updates(ui: &Rc<Ui>) {
    let ui = ui.clone();
    let rx = ui.client.updates.clone();
    glib::spawn_future_local(async move {
        while let Ok(update) = rx.recv().await {
            match update {
                DaemonUpdate::State(state) => {
                    *ui.state.borrow_mut() = state;
                    *ui.available.borrow_mut() = true;
                    ui.set_busy(false);
                    ui.refresh();
                }
                DaemonUpdate::Error { message } => {
                    ui.set_busy(false);
                    ui.toast(&message);
                }
                DaemonUpdate::Unreachable(reason) => {
                    *ui.available.borrow_mut() = false;
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
        let desc = match reason {
            UnreachableReason::NotRunning => {
                "BootDrive could not reach usb-signaller on the system bus."
            }
            UnreachableReason::NoMassStorage => {
                "Your usb-signaller has no mass-storage mode yet. Install the patched build."
            }
            UnreachableReason::Other(_) => "BootDrive could not talk to usb-signaller.",
        };
        self.setup_status.set_description(Some(desc));
        self.status_row.set_title("Unavailable");
        self.status_row.set_subtitle("USB status");
        self.content_stack.set_visible_child_name("setup");
    }

    fn refresh(&self) {
        // Sidebar status line.
        let state = *self.state.borrow();
        if *self.available.borrow() {
            self.status_row.set_title(state.headline());
            self.status_row.set_subtitle(match state {
                DriveState::Active => "exposed over USB",
                _ => "ready",
            });
        }

        // Content.
        if !*self.available.borrow() {
            // setup page already shown by show_setup
            return;
        }

        let sel = *self.selected.borrow();
        let entry = sel.and_then(|i| self.library.borrow().entries.get(i).cloned());

        match entry {
            Some(entry) => {
                self.content_title.set_title(&entry.display_name);
                self.content_title
                    .set_subtitle(if state == DriveState::Active {
                        "exposed over USB"
                    } else {
                        "ready to expose"
                    });
                self.size_row.set_subtitle(
                    &entry
                        .size
                        .map(human_size)
                        .unwrap_or_else(|| "unknown".to_string()),
                );
                self.mode_row.set_visible(entry.hybrid);
                self.mode_row.set_selected(if entry.cdrom { 0 } else { 1 });
                self.path_row.set_subtitle(&entry.path);
                self.image_group.set_visible(true);
                self.content_stack.set_visible_child_name("detail");
            }
            None => {
                self.content_title.set_title("BootDrive");
                self.content_title.set_subtitle("");
                self.content_stack.set_visible_child_name("empty");
            }
        }

        self.refresh_controls();
    }

    fn refresh_controls(&self) {
        let state = *self.state.borrow();
        let busy = *self.busy.borrow();
        let has_selection = self.selected.borrow().is_some();
        let transitioning = matches!(state, DriveState::Preparing | DriveState::Ejecting);
        let enabled = !busy && !transitioning && *self.available.borrow();

        self.mode_row
            .set_sensitive(enabled && state != DriveState::Active);

        match state {
            DriveState::Active => {
                self.primary_button.set_label("Eject");
                self.primary_button
                    .set_css_classes(&["destructive-action", "pill"]);
                self.primary_button.set_sensitive(enabled);
            }
            _ => {
                self.primary_button.set_label("Expose over USB");
                self.primary_button
                    .set_css_classes(&["suggested-action", "pill"]);
                self.primary_button
                    .set_sensitive(enabled && has_selection && !transitioning);
            }
        }
    }
}
