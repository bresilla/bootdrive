//! The BootDrive window.
//!
//! An `AdwOverlaySplitView` with a hamburger-toggled sidebar of tabs:
//! **Mount** (the main view — USB status + expose/eject) and **Images** (the
//! ISO library). Adaptive: on the phone the sidebar becomes an overlay. State
//! comes from usb-signaller via [`DaemonClient`].

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use bootdrive_common::DriveState;
use gtk4 as gtk;
use libadwaita as adw;

use adw::prelude::*;

use crate::library::{human_size, import, ImageEntry, Library};
use crate::usb_moded::{DaemonClient, DaemonCommand, DaemonUpdate, UnreachableReason};

/// Shared UI state.
struct Ui {
    window: adw::ApplicationWindow,
    overlay: adw::OverlaySplitView,
    toasts: adw::ToastOverlay,
    content_title: adw::WindowTitle,
    view_stack: gtk::Stack,

    // Mount view.
    mount_stack: gtk::Stack,
    mount_status_title: gtk::Label,
    mount_status_detail: gtk::Label,
    mount_image_row: adw::ActionRow,
    mount_mode_row: adw::ComboRow,
    primary_button: gtk::Button,
    setup_status: adw::StatusPage,

    // Images view.
    images_list: gtk::ListBox,
    images_group: adw::PreferencesGroup,

    client: DaemonClient,
    library: RefCell<Library>,
    active: RefCell<Option<usize>>,
    state: RefCell<DriveState>,
    busy: RefCell<bool>,
    available: RefCell<bool>,
}

const TAB_MOUNT: &str = "mount";
const TAB_IMAGES: &str = "images";

/// Build and present the main window.
pub fn build(app: &adw::Application) -> adw::ApplicationWindow {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("BootDrive")
        .default_width(860)
        .default_height(600)
        .width_request(320)
        .height_request(320)
        .build();

    let toasts = adw::ToastOverlay::new();

    // ---- Sidebar (tabs + menu) -------------------------------------------
    let menu = gio::Menu::new();
    menu.append(Some("About BootDrive"), Some("win.about"));
    let menu_button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text("Main menu")
        .menu_model(&menu)
        .build();
    let sidebar_header = adw::HeaderBar::builder().show_title(false).build();
    sidebar_header.pack_end(&menu_button);

    let nav_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .css_classes(vec!["navigation-sidebar".to_string()])
        .build();
    nav_list.append(&nav_row("Mount", "drive-removable-media-symbolic"));
    nav_list.append(&nav_row("Images", "media-optical-symbolic"));

    let sidebar_toolbar = adw::ToolbarView::new();
    sidebar_toolbar.add_top_bar(&sidebar_header);
    sidebar_toolbar.set_content(Some(&nav_list));
    sidebar_toolbar.add_css_class("sidebar-pane");

    // ---- Content ----------------------------------------------------------
    let sidebar_toggle = gtk::ToggleButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text("Show tabs")
        .active(true)
        .build();
    let add_button = gtk::Button::from_icon_name("list-add-symbolic");
    add_button.set_tooltip_text(Some("Add an image"));
    add_button.set_visible(false);

    let content_title = adw::WindowTitle::new("Mount", "");
    let content_header = adw::HeaderBar::new();
    content_header.set_title_widget(Some(&content_title));
    content_header.pack_start(&sidebar_toggle);
    content_header.pack_end(&add_button);

    let view_stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .build();
    let (mount_page, mount_widgets) = mount_page();
    view_stack.add_named(&mount_page, Some(TAB_MOUNT));
    let (images_page, images_list, images_group) = images_page(&add_button);
    view_stack.add_named(&images_page, Some(TAB_IMAGES));
    view_stack.set_visible_child_name(TAB_MOUNT);

    let content_toolbar = adw::ToolbarView::new();
    content_toolbar.add_top_bar(&content_header);
    content_toolbar.set_content(Some(&view_stack));

    // ---- Overlay split view + adaptive breakpoint -------------------------
    let overlay = adw::OverlaySplitView::builder()
        .sidebar(&sidebar_toolbar)
        .content(&content_toolbar)
        .max_sidebar_width(240.0)
        .build();
    overlay
        .bind_property("show-sidebar", &sidebar_toggle, "active")
        .bidirectional()
        .sync_create()
        .build();
    toasts.set_child(Some(&overlay));
    window.set_content(Some(&toasts));

    if let Ok(cond) = adw::BreakpointCondition::parse("max-width: 550sp") {
        let bp = adw::Breakpoint::new(cond);
        bp.add_setter(&overlay, "collapsed", Some(&true.to_value()));
        window.add_breakpoint(bp);
    }

    let ui = Rc::new(Ui {
        window: window.clone(),
        overlay,
        toasts,
        content_title,
        view_stack,
        mount_stack: mount_widgets.stack,
        mount_status_title: mount_widgets.status_title,
        mount_status_detail: mount_widgets.status_detail,
        mount_image_row: mount_widgets.image_row,
        mount_mode_row: mount_widgets.mode_row,
        primary_button: mount_widgets.primary_button,
        setup_status: mount_widgets.setup_status,
        images_list,
        images_group,
        client: DaemonClient::spawn(),
        library: RefCell::new(Library::load()),
        active: RefCell::new(None),
        state: RefCell::new(DriveState::Unavailable),
        busy: RefCell::new(false),
        available: RefCell::new(false),
    });

    install_about_action(&ui);
    wire(&ui, &nav_list, &add_button);
    // Select the Mount tab by default.
    nav_list.select_row(nav_list.row_at_index(0).as_ref());
    rebuild_list(&ui);
    listen_for_updates(&ui);
    ui.refresh();

    window
}

fn nav_row(title: &str, icon: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(title).build();
    row.add_prefix(&gtk::Image::from_icon_name(icon));
    row
}

/// Widgets we keep from the Mount page.
struct MountWidgets {
    stack: gtk::Stack,
    status_title: gtk::Label,
    status_detail: gtk::Label,
    image_row: adw::ActionRow,
    mode_row: adw::ComboRow,
    primary_button: gtk::Button,
    setup_status: adw::StatusPage,
}

/// Build the Mount page and return it plus the widgets we update.
fn mount_page() -> (gtk::Widget, MountWidgets) {
    let page = adw::PreferencesPage::new();

    // USB status group.
    let status_group = adw::PreferencesGroup::new();
    let status_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .margin_top(4)
        .margin_bottom(4)
        .build();
    let status_title = gtk::Label::builder()
        .halign(gtk::Align::Start)
        .css_classes(vec!["title-1".to_string()])
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

    // Selected image group.
    let image_group = adw::PreferencesGroup::builder().title("Image").build();
    let image_row = adw::ActionRow::builder()
        .title("No image selected")
        .subtitle("Add one in the Images tab")
        .build();
    image_row.add_prefix(&gtk::Image::from_icon_name(
        "drive-removable-media-symbolic",
    ));
    let mode_model = gtk::StringList::new(&["USB CD-ROM", "USB disk"]);
    let mode_row = adw::ComboRow::builder()
        .title("Exposure mode")
        .subtitle("How the computer sees it")
        .model(&mode_model)
        .build();
    image_group.add(&image_row);
    image_group.add(&mode_row);

    // Action button.
    let primary_button = gtk::Button::builder()
        .label("Expose over USB")
        .css_classes(vec!["suggested-action".to_string(), "pill".to_string()])
        .halign(gtk::Align::Center)
        .margin_top(6)
        .build();
    let button_group = adw::PreferencesGroup::new();
    button_group.add(
        &adw::Clamp::builder()
            .maximum_size(360)
            .child(&primary_button)
            .build(),
    );

    page.add(&status_group);
    page.add(&image_group);
    page.add(&button_group);

    // Setup page (usb-signaller unavailable).
    let setup_status = adw::StatusPage::builder()
        .icon_name("dialog-warning-symbolic")
        .title("usb-signaller unavailable")
        .build();

    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .build();
    stack.add_named(&page, Some("ready"));
    stack.add_named(&setup_status, Some("setup"));
    stack.set_visible_child_name("ready");

    (
        stack.clone().upcast(),
        MountWidgets {
            stack,
            status_title,
            status_detail,
            image_row,
            mode_row,
            primary_button,
            setup_status,
        },
    )
}

/// Build the Images page and return it plus the list box.
fn images_page(add_button: &gtk::Button) -> (gtk::Widget, gtk::ListBox, adw::PreferencesGroup) {
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::builder()
        .title("Images")
        .description("Stored in BootDrive's library")
        .build();
    group.set_header_suffix(Some(add_button));
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(vec!["boxed-list".to_string()])
        .build();
    group.add(&list);
    page.add(&group);
    (page.upcast(), list, group)
}

fn install_about_action(ui: &Rc<Ui>) {
    let action = gio::SimpleAction::new("about", None);
    let window = ui.window.clone();
    action.connect_activate(move |_, _| present_about(&window));
    ui.window.add_action(&action);
}

/// A compact, mobile-dismissable About dialog (`AdwDialog`).
fn present_about(window: &adw::ApplicationWindow) {
    let dialog = adw::Dialog::builder()
        .title("About")
        .content_width(360)
        .build();

    let header = adw::HeaderBar::new();
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(12)
        .margin_bottom(28)
        .margin_start(24)
        .margin_end(24)
        .halign(gtk::Align::Center)
        .build();
    let icon = gtk::Image::from_icon_name("net.bresilla.BootDrive");
    icon.set_pixel_size(96);
    let name = gtk::Label::builder()
        .label("BootDrive")
        .css_classes(vec!["title-1".to_string()])
        .build();
    let version = gtk::Label::builder()
        .label("0.1.0")
        .css_classes(vec!["dim-label".to_string()])
        .margin_bottom(6)
        .build();
    let desc = gtk::Label::builder()
        .label("Expose a disk image as a bootable USB drive.")
        .wrap(true)
        .justify(gtk::Justification::Center)
        .margin_bottom(6)
        .build();
    let dev = gtk::Label::builder()
        .label("Trim Bresilla")
        .css_classes(vec!["dim-label".to_string()])
        .build();
    let link = gtk::LinkButton::with_label("https://github.com/bresilla/bootdrive", "Project page");
    for w in [
        icon.upcast_ref::<gtk::Widget>(),
        name.upcast_ref(),
        version.upcast_ref(),
        desc.upcast_ref(),
        dev.upcast_ref(),
        link.upcast_ref(),
    ] {
        body.append(w);
    }

    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&header);
    tv.set_content(Some(&body));
    dialog.set_child(Some(&tv));
    dialog.present(Some(window));
}

fn wire(ui: &Rc<Ui>, nav_list: &gtk::ListBox, add_button: &gtk::Button) {
    // Sidebar tab selection.
    {
        let ui = ui.clone();
        let add_button = add_button.clone();
        nav_list.connect_row_selected(move |_, row| {
            let Some(row) = row else { return };
            let (name, title, adding) = match row.index() {
                1 => (TAB_IMAGES, "Images", true),
                _ => (TAB_MOUNT, "Mount", false),
            };
            ui.view_stack.set_visible_child_name(name);
            ui.content_title.set_title(title);
            // The add button only belongs to the Images tab.
            add_button.set_visible(adding);
            if ui.overlay.is_collapsed() {
                ui.overlay.set_show_sidebar(false);
            }
        });
    }
    // Add image.
    {
        let ui = ui.clone();
        add_button.connect_clicked(move |_| choose_image(&ui));
    }
    // Mode combo.
    {
        let ui = ui.clone();
        let row = ui.mount_mode_row.clone();
        row.connect_selected_notify(move |row| {
            if let Some(i) = *ui.active.borrow() {
                let mut lib = ui.library.borrow_mut();
                if let Some(e) = lib.entries.get_mut(i) {
                    e.cdrom = row.selected() == 0;
                    lib.save();
                }
            }
        });
    }
    // Expose / eject.
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

/// Rebuild the Images list from the library.
fn rebuild_list(ui: &Rc<Ui>) {
    while let Some(child) = ui.images_list.first_child() {
        ui.images_list.remove(&child);
    }
    let lib = ui.library.borrow();
    ui.images_group
        .set_description(Some(&match lib.entries.len() {
            0 => "Stored in BootDrive's library".to_string(),
            1 => format!("1 image · {} used", human_size(lib.total_size())),
            n => format!("{n} images · {} used", human_size(lib.total_size())),
        }));
    if lib.entries.is_empty() {
        let placeholder = adw::ActionRow::builder()
            .title("No images yet")
            .subtitle("Tap + to add an .iso, .img or .raw")
            .build();
        placeholder.add_css_class("dim-label");
        ui.images_list.append(&placeholder);
        return;
    }
    let active = *ui.active.borrow();
    for (i, entry) in lib.entries.iter().enumerate() {
        let row = adw::ActionRow::builder()
            .title(&entry.display_name)
            .subtitle(subtitle_for(entry))
            .activatable(true)
            .build();
        row.add_prefix(&gtk::Image::from_icon_name(if entry.cdrom {
            "media-optical-symbolic"
        } else {
            "drive-harddisk-symbolic"
        }));
        if Some(i) == active {
            row.add_suffix(&gtk::Image::from_icon_name("object-select-symbolic"));
        }
        if !Library::exists(entry) {
            row.add_css_class("dim-label");
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
        {
            let ui = ui.clone();
            let row_weak = row.downgrade();
            row.connect_activated(move |_| {
                if let Some(row) = row_weak.upgrade() {
                    select_active(&ui, row.index() as usize);
                }
            });
        }
        ui.images_list.append(&row);
    }
}

fn subtitle_for(entry: &ImageEntry) -> String {
    if !Library::exists(entry) {
        return "missing — file not found".to_string();
    }
    let size = entry
        .size
        .map(human_size)
        .unwrap_or_else(|| "unknown size".to_string());
    format!("{} · {}", size, entry.mode().label())
}

/// Make the image at `index` the active one and jump to the Mount tab.
fn select_active(ui: &Rc<Ui>, index: usize) {
    if index >= ui.library.borrow().entries.len() {
        return;
    }
    *ui.active.borrow_mut() = Some(index);
    ui.view_stack.set_visible_child_name(TAB_MOUNT);
    ui.content_title.set_title("Mount");
    rebuild_list(ui);
    ui.refresh();
}

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
                    start_import(&ui, path);
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

/// Messages from the copy thread to the GTK loop.
enum ImportMsg {
    Progress(u64, u64),
    Done(ImageEntry),
    Cancelled,
    Failed(String),
}

/// Copy `source` into the library with a progress dialog.
fn start_import(ui: &Rc<Ui>, source: PathBuf) {
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image".to_string());

    let dialog = adw::Dialog::builder()
        .title("Importing")
        .content_width(380)
        .can_close(false)
        .build();
    let header = adw::HeaderBar::builder()
        .show_end_title_buttons(false)
        .show_start_title_buttons(false)
        .build();
    let bar = gtk::ProgressBar::builder()
        .show_text(true)
        .text("Preparing…")
        .build();
    let label = gtk::Label::builder()
        .label(format!("Copying “{name}” into your library…"))
        .wrap(true)
        .justify(gtk::Justification::Center)
        .build();
    let cancel = gtk::Button::builder()
        .label("Cancel")
        .halign(gtk::Align::Center)
        .build();
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .margin_top(12)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .build();
    body.append(&label);
    body.append(&bar);
    body.append(&cancel);
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&header);
    tv.set_content(Some(&body));
    dialog.set_child(Some(&tv));
    dialog.present(Some(&ui.window));

    let cancel_flag = Arc::new(AtomicBool::new(false));
    {
        let cancel_flag = cancel_flag.clone();
        cancel.connect_clicked(move |b| {
            b.set_sensitive(false);
            cancel_flag.store(true, Ordering::Relaxed);
        });
    }

    let (tx, rx) = async_channel::unbounded::<ImportMsg>();
    {
        let cancel_flag = cancel_flag.clone();
        thread::spawn(move || {
            let result = import(
                &source,
                |(c, t)| {
                    let _ = tx.send_blocking(ImportMsg::Progress(c, t));
                },
                &cancel_flag,
            );
            let msg = match result {
                Ok(entry) => ImportMsg::Done(entry),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => ImportMsg::Cancelled,
                Err(e) => ImportMsg::Failed(e.to_string()),
            };
            let _ = tx.send_blocking(msg);
        });
    }

    let ui = ui.clone();
    glib::spawn_future_local(async move {
        while let Ok(msg) = rx.recv().await {
            match msg {
                ImportMsg::Progress(c, t) => {
                    bar.set_fraction(if t > 0 { c as f64 / t as f64 } else { 0.0 });
                    bar.set_text(Some(&format!("{} / {}", human_size(c), human_size(t))));
                }
                ImportMsg::Done(entry) => {
                    dialog.force_close();
                    ui.library.borrow_mut().add(entry);
                    rebuild_list(&ui);
                    let last = ui.library.borrow().entries.len().saturating_sub(1);
                    select_active(&ui, last);
                    ui.toast("Added to your library.");
                    break;
                }
                ImportMsg::Cancelled => {
                    dialog.force_close();
                    break;
                }
                ImportMsg::Failed(m) => {
                    dialog.force_close();
                    ui.toast(&format!("Import failed: {m}"));
                    break;
                }
            }
        }
    });
}

fn remove_image(ui: &Rc<Ui>, index: usize) {
    ui.library.borrow_mut().remove(index);
    {
        let mut active = ui.active.borrow_mut();
        match *active {
            Some(i) if i == index => *active = None,
            Some(i) if i > index => *active = Some(i - 1),
            _ => {}
        }
    }
    rebuild_list(ui);
    ui.refresh();
}

fn activate(ui: &Rc<Ui>) {
    let Some(i) = *ui.active.borrow() else {
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
        self.mount_stack.set_visible_child_name("setup");
    }

    fn refresh(&self) {
        let state = *self.state.borrow();
        let available = *self.available.borrow();

        if !available {
            self.mount_stack.set_visible_child_name("setup");
            return;
        }
        self.mount_stack.set_visible_child_name("ready");

        self.mount_status_title.set_text(state.headline());

        let active = *self.active.borrow();
        let entry = active.and_then(|i| self.library.borrow().entries.get(i).cloned());

        self.mount_status_detail
            .set_text(&detail_text(state, entry.as_ref()));

        match entry {
            Some(entry) => {
                self.mount_image_row.set_title(&entry.display_name);
                let size = entry
                    .size
                    .map(human_size)
                    .unwrap_or_else(|| "unknown size".to_string());
                self.mount_image_row
                    .set_subtitle(&format!("{} · {}", size, entry.mode().label()));
                self.mount_mode_row.set_visible(entry.hybrid);
                self.mount_mode_row
                    .set_selected(if entry.cdrom { 0 } else { 1 });
            }
            None => {
                self.mount_image_row.set_title("No image selected");
                self.mount_image_row
                    .set_subtitle("Add one in the Images tab");
                self.mount_mode_row.set_visible(false);
            }
        }

        self.refresh_controls();
    }

    fn refresh_controls(&self) {
        let state = *self.state.borrow();
        let busy = *self.busy.borrow();
        let has_active = self.active.borrow().is_some();
        let transitioning = matches!(state, DriveState::Preparing | DriveState::Ejecting);
        let enabled = !busy && !transitioning && *self.available.borrow();

        self.mount_mode_row
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
                    .set_sensitive(enabled && has_active && !transitioning);
            }
        }
    }
}

fn detail_text(state: DriveState, entry: Option<&ImageEntry>) -> String {
    match state {
        DriveState::Unavailable => "usb-signaller is unavailable.".to_string(),
        DriveState::Preparing => "Setting up the USB gadget…".to_string(),
        DriveState::Ejecting => "Returning to normal USB behaviour…".to_string(),
        DriveState::Active => match entry.map(|e| e.mode()) {
            Some(bootdrive_common::ImageMode::Disk) => {
                "Connected as a bootable USB disk.".to_string()
            }
            _ => "Connected as a bootable CD-ROM.".to_string(),
        },
        DriveState::Idle | DriveState::Error => {
            if entry.is_some() {
                "Ready to expose over USB.".to_string()
            } else {
                "Add and select an image to get started.".to_string()
            }
        }
    }
}
