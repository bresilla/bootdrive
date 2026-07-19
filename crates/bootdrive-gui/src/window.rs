//! The BootDrive window.
//!
//! An `AdwOverlaySplitView` with a hamburger-toggled sidebar of tabs:
//! **Mount** (the main view: pick a library image and expose it over USB) and
//! **Images** (manage the ISO library: add / remove). The two are kept
//! separate on purpose: the Images tab never arms anything, it only manages
//! files; choosing what to expose happens in Mount. State comes from
//! usb-signaller via [`DaemonClient`].

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use bootdrive_common::DriveState;
use gtk4 as gtk;
use libadwaita as adw;

use adw::prelude::*;

use crate::catalog;
use crate::library::{download, human_size, import, ImageEntry, Library};
use crate::usb_moded::{DaemonClient, DaemonCommand, DaemonUpdate, UnreachableReason};

/// A little accent stylesheet so the app isn't all grey.
const CSS: &str = "
.bd-hero {
  padding: 24px 18px;
  border-radius: 22px;
  background: alpha(@window_fg_color, 0.045);
}
.bd-hero.busy   { background: alpha(@accent_bg_color, 0.14); }
.bd-hero.active { background: alpha(@success_color, 0.14); }
.bd-hero.error  { background: alpha(@error_color, 0.13); }

.bd-badge {
  border-radius: 999px;
  background: alpha(@window_fg_color, 0.09);
  color: @window_fg_color;
}
.bd-badge.busy   { background: alpha(@accent_bg_color, 0.22); color: @accent_fg_color; }
.bd-badge.active { background: alpha(@success_color, 0.24); color: @success_color; }
.bd-badge.error  { background: alpha(@error_color, 0.22);  color: @error_color; }

.bd-hero-title { font-weight: 800; font-size: 1.35rem; }

.bd-cd   { color: @accent_color; }
.bd-disk { color: @success_color; }
";

/// Shared UI state.
struct Ui {
    window: adw::ApplicationWindow,
    overlay: adw::OverlaySplitView,
    toasts: adw::ToastOverlay,
    content_title: adw::WindowTitle,
    view_stack: gtk::Stack,

    // Mount view.
    mount_stack: gtk::Stack,
    hero: gtk::Box,
    badge: gtk::Box,
    badge_icon: gtk::Image,
    status_title: gtk::Label,
    status_detail: gtk::Label,
    image_row: adw::ActionRow,
    mode_row: adw::ComboRow,
    primary_button: gtk::Button,
    setup_status: adw::StatusPage,

    // Images view.
    images_list: gtk::ListBox,
    images_group: adw::PreferencesGroup,

    // Download view.
    download_stack: gtk::Stack,
    download_list: gtk::ListBox,
    catalog_loaded: RefCell<bool>,

    client: DaemonClient,
    library: RefCell<Library>,
    active: RefCell<Option<usize>>,
    state: RefCell<DriveState>,
    busy: RefCell<bool>,
    available: RefCell<bool>,
}

const TAB_DOWNLOAD: &str = "download";
const TAB_MOUNT: &str = "mount";
const TAB_IMAGES: &str = "images";

/// Build and present the main window.
pub fn build(app: &adw::Application) -> adw::ApplicationWindow {
    load_css();

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
    nav_list.append(&nav_row("Download", "folder-download-symbolic"));

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
    let (download_page, download_stack, download_list) = download_page();
    view_stack.add_named(&download_page, Some(TAB_DOWNLOAD));
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
        hero: mount_widgets.hero,
        badge: mount_widgets.badge,
        badge_icon: mount_widgets.badge_icon,
        status_title: mount_widgets.status_title,
        status_detail: mount_widgets.status_detail,
        image_row: mount_widgets.image_row,
        mode_row: mount_widgets.mode_row,
        primary_button: mount_widgets.primary_button,
        setup_status: mount_widgets.setup_status,
        images_list,
        images_group,
        download_stack,
        download_list,
        catalog_loaded: RefCell::new(false),
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

fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_data(CSS);
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn nav_row(title: &str, icon: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(title).build();
    row.add_prefix(&gtk::Image::from_icon_name(icon));
    row
}

/// Widgets we keep from the Mount page.
struct MountWidgets {
    stack: gtk::Stack,
    hero: gtk::Box,
    badge: gtk::Box,
    badge_icon: gtk::Image,
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

    // Colored status hero.
    let hero = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .css_classes(vec!["bd-hero".to_string()])
        .build();
    let badge = gtk::Box::builder()
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .hexpand(false)
        .vexpand(false)
        .width_request(92)
        .height_request(92)
        .css_classes(vec!["bd-badge".to_string()])
        .build();
    let badge_icon = gtk::Image::from_icon_name("drive-removable-media-symbolic");
    badge_icon.set_pixel_size(46);
    badge_icon.set_halign(gtk::Align::Center);
    badge_icon.set_valign(gtk::Align::Center);
    badge_icon.set_hexpand(true);
    badge.append(&badge_icon);
    let status_title = gtk::Label::builder()
        .halign(gtk::Align::Center)
        .css_classes(vec!["bd-hero-title".to_string()])
        .label("Connecting…")
        .build();
    let status_detail = gtk::Label::builder()
        .halign(gtk::Align::Center)
        .justify(gtk::Justification::Center)
        .wrap(true)
        .css_classes(vec!["dim-label".to_string()])
        .label("")
        .build();
    hero.append(&badge);
    hero.append(&status_title);
    hero.append(&status_detail);
    let hero_group = adw::PreferencesGroup::new();
    hero_group.add(&hero);

    // Image selection group (this is where you pick from the library).
    let image_group = adw::PreferencesGroup::builder().title("Image").build();
    let image_row = adw::ActionRow::builder()
        .title("Choose an image")
        .subtitle("Pick one from your library")
        .activatable(true)
        .build();
    image_row.add_prefix(&gtk::Image::from_icon_name(
        "drive-removable-media-symbolic",
    ));
    image_row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
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
        .label("Choose an image")
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

    page.add(&hero_group);
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
            hero,
            badge,
            badge_icon,
            status_title,
            status_detail,
            image_row,
            mode_row,
            primary_button,
            setup_status,
        },
    )
}

/// Build the Images page (management only) and return it plus the list box.
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

/// Build the Download page: a search box over a list of distros, plus loading
/// and empty states. Returns the outer widget, its stack, and the distro list.
fn download_page() -> (gtk::Widget, gtk::Stack, gtk::ListBox) {
    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .build();

    let spinner = gtk::Spinner::builder()
        .spinning(true)
        .width_request(32)
        .height_request(32)
        .build();
    let loading = adw::StatusPage::builder().title("Reading catalogue…").build();
    loading.set_child(Some(&spinner));
    stack.add_named(&loading, Some("loading"));

    let empty = adw::StatusPage::builder()
        .icon_name("folder-download-symbolic")
        .title("No catalogue")
        .description("osinfo-db is not installed, so there are no distros to list.")
        .build();
    stack.add_named(&empty, Some("empty"));

    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search distros")
        .margin_top(8)
        .margin_start(12)
        .margin_end(12)
        .margin_bottom(4)
        .build();
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(vec!["boxed-list".to_string()])
        .valign(gtk::Align::Start)
        .build();
    {
        let search = search.clone();
        list.set_filter_func(move |row| {
            let q = search.text().to_lowercase();
            if q.is_empty() {
                return true;
            }
            row.clone()
                .downcast::<adw::ExpanderRow>()
                .map(|e| e.title().to_lowercase().contains(&q))
                .unwrap_or(true)
        });
    }
    {
        let list = list.clone();
        search.connect_search_changed(move |_| list.invalidate_filter());
    }
    let clamp = adw::Clamp::builder()
        .maximum_size(640)
        .child(&list)
        .margin_top(8)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&clamp)
        .build();
    let ready = gtk::Box::new(gtk::Orientation::Vertical, 0);
    ready.append(&search);
    ready.append(&scroll);
    stack.add_named(&ready, Some("ready"));

    stack.set_visible_child_name("loading");
    (stack.clone().upcast(), stack, list)
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
                2 => (TAB_DOWNLOAD, "Download", false),
                _ => (TAB_MOUNT, "Mount", false),
            };
            ui.view_stack.set_visible_child_name(name);
            ui.content_title.set_title(title);
            // The add button only belongs to the Images tab.
            add_button.set_visible(adding);
            // Read the catalogue the first time the Download tab is opened.
            if name == TAB_DOWNLOAD {
                load_catalog(&ui);
            }
            if ui.overlay.is_collapsed() {
                ui.overlay.set_show_sidebar(false);
            }
        });
    }
    // Add image (from the Images tab: manage only, do not arm it).
    {
        let ui = ui.clone();
        add_button.connect_clicked(move |_| choose_image(&ui, false));
    }
    // Tapping the image row in Mount opens the library picker.
    {
        let ui = ui.clone();
        let row = ui.image_row.clone();
        row.connect_activated(move |_| choose_active(&ui));
    }
    // Mode combo.
    {
        let ui = ui.clone();
        let row = ui.mode_row.clone();
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
    // Primary button: pick → expose → eject depending on state.
    {
        let ui = ui.clone();
        let button = ui.primary_button.clone();
        button.connect_clicked(move |_| {
            let state = *ui.state.borrow();
            match state {
                DriveState::Active => confirm_and_eject(&ui),
                DriveState::Idle | DriveState::Error => {
                    if ui.active.borrow().is_none() {
                        choose_active(&ui);
                    } else {
                        activate(&ui);
                    }
                }
                _ => {}
            }
        });
    }
}

/// Rebuild the Images list from the library. Rows are informational only;
/// they never arm the Mount tab.
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
    for (i, entry) in lib.entries.iter().enumerate() {
        let row = adw::ActionRow::builder()
            .title(&entry.display_name)
            .subtitle(subtitle_for(entry))
            .build();
        let icon = gtk::Image::from_icon_name(if entry.cdrom {
            "media-optical-symbolic"
        } else {
            "drive-harddisk-symbolic"
        });
        icon.add_css_class(if entry.cdrom { "bd-cd" } else { "bd-disk" });
        row.add_prefix(&icon);
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
            remove_btn.connect_clicked(move |_| remove_image(&ui, i));
        }
        ui.images_list.append(&row);
    }
}

fn subtitle_for(entry: &ImageEntry) -> String {
    if !Library::exists(entry) {
        return "missing, file not found".to_string();
    }
    let size = entry
        .size
        .map(human_size)
        .unwrap_or_else(|| "unknown size".to_string());
    format!("{} · {}", size, entry.mode().label())
}

/// Open a dialog to pick which library image the Mount tab should expose.
fn choose_active(ui: &Rc<Ui>) {
    let dialog = adw::Dialog::builder()
        .title("Choose an image")
        .content_width(420)
        .content_height(480)
        .build();
    let header = adw::HeaderBar::new();
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&header);

    let lib = ui.library.borrow();
    if lib.entries.is_empty() {
        let status = adw::StatusPage::builder()
            .icon_name("drive-removable-media-symbolic")
            .title("No images yet")
            .description("Add a disk image to expose it over USB.")
            .build();
        let add = gtk::Button::builder()
            .label("Add an image…")
            .halign(gtk::Align::Center)
            .css_classes(vec!["suggested-action".to_string(), "pill".to_string()])
            .build();
        {
            let ui = ui.clone();
            let dialog = dialog.clone();
            add.connect_clicked(move |_| {
                dialog.close();
                choose_image(&ui, true);
            });
        }
        status.set_child(Some(&add));
        tv.set_content(Some(&status));
        dialog.set_child(Some(&tv));
        dialog.present(Some(&ui.window));
        return;
    }

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(vec!["boxed-list".to_string()])
        .build();
    let active = *ui.active.borrow();
    for (i, entry) in lib.entries.iter().enumerate() {
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
        icon.add_css_class(if entry.cdrom { "bd-cd" } else { "bd-disk" });
        row.add_prefix(&icon);
        if Some(i) == active {
            let check = gtk::Image::from_icon_name("object-select-symbolic");
            check.add_css_class("accent");
            row.add_suffix(&check);
        }
        row.set_sensitive(Library::exists(entry));
        {
            let ui = ui.clone();
            let dialog = dialog.clone();
            row.connect_activated(move |_| {
                select_active(&ui, i);
                dialog.close();
            });
        }
        list.append(&row);
    }

    let group = adw::PreferencesGroup::new();
    group.add(&list);
    let clamped = adw::PreferencesPage::new();
    clamped.add(&group);
    tv.set_content(Some(&clamped));
    dialog.set_child(Some(&tv));
    dialog.present(Some(&ui.window));
}

/// Make the image at `index` the one Mount will expose.
fn select_active(ui: &Rc<Ui>, index: usize) {
    if index >= ui.library.borrow().entries.len() {
        return;
    }
    *ui.active.borrow_mut() = Some(index);
    rebuild_list(ui);
    ui.refresh();
}

fn choose_image(ui: &Rc<Ui>, select_after: bool) {
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
                    start_import(&ui, path, select_after);
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

/// Run a background transfer that produces an [`ImageEntry`], behind a progress
/// dialog with a Cancel button. `spawn_worker` gets the progress sender and the
/// cancel flag and must do the work on its own thread, sending one terminal
/// message when it finishes. `verb` names the action for the failure toast.
fn run_transfer<S>(
    ui: &Rc<Ui>,
    title: &str,
    message: String,
    verb: &'static str,
    select_after: bool,
    spawn_worker: S,
) where
    S: FnOnce(async_channel::Sender<ImportMsg>, Arc<AtomicBool>),
{
    let dialog = adw::Dialog::builder()
        .title(title)
        .content_width(380)
        .can_close(false)
        .build();
    let header = adw::HeaderBar::builder()
        .show_end_title_buttons(false)
        .show_start_title_buttons(false)
        .build();
    let bar = gtk::ProgressBar::builder()
        .show_text(true)
        .text("Preparing")
        .build();
    let label = gtk::Label::builder()
        .label(message)
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
    spawn_worker(tx, cancel_flag);

    let ui = ui.clone();
    glib::spawn_future_local(async move {
        while let Ok(msg) = rx.recv().await {
            match msg {
                ImportMsg::Progress(c, t) => {
                    bar.set_fraction(if t > 0 { c as f64 / t as f64 } else { 0.0 });
                    if t > 0 {
                        bar.set_text(Some(&format!("{} / {}", human_size(c), human_size(t))));
                    } else {
                        bar.set_text(Some(&human_size(c)));
                    }
                }
                ImportMsg::Done(entry) => {
                    dialog.force_close();
                    ui.library.borrow_mut().add(entry);
                    rebuild_list(&ui);
                    if select_after {
                        let last = ui.library.borrow().entries.len().saturating_sub(1);
                        select_active(&ui, last);
                    }
                    ui.toast("Added to your library.");
                    break;
                }
                ImportMsg::Cancelled => {
                    dialog.force_close();
                    break;
                }
                ImportMsg::Failed(m) => {
                    dialog.force_close();
                    ui.toast(&format!("{verb} failed: {m}"));
                    break;
                }
            }
        }
    });
}

/// Turn a transfer result into the message that ends the progress loop.
fn terminal_msg(result: std::io::Result<ImageEntry>) -> ImportMsg {
    match result {
        Ok(entry) => ImportMsg::Done(entry),
        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => ImportMsg::Cancelled,
        Err(e) => ImportMsg::Failed(e.to_string()),
    }
}

/// Copy `source` into the library. With `select_after`, the imported image also
/// becomes the Mount selection.
fn start_import(ui: &Rc<Ui>, source: PathBuf, select_after: bool) {
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image".to_string());
    run_transfer(
        ui,
        "Importing",
        format!("Copying {name} into your library."),
        "Import",
        select_after,
        move |tx, cancel| {
            thread::spawn(move || {
                let result = import(
                    &source,
                    |(c, t)| {
                        let _ = tx.send_blocking(ImportMsg::Progress(c, t));
                    },
                    &cancel,
                );
                let _ = tx.send_blocking(terminal_msg(result));
            });
        },
    );
}

/// Download `img` from its catalogue URL into the library.
fn start_download(ui: &Rc<Ui>, img: &catalog::Image) {
    let url = img.url.clone();
    let name = img.os_name.clone();
    run_transfer(
        ui,
        "Downloading",
        format!("Downloading {name}."),
        "Download",
        false,
        move |tx, cancel| {
            thread::spawn(move || {
                let result = download(
                    &url,
                    |(c, t)| {
                        let _ = tx.send_blocking(ImportMsg::Progress(c, t));
                    },
                    &cancel,
                );
                let _ = tx.send_blocking(terminal_msg(result));
            });
        },
    );
}

/// Read osinfo-db in the background the first time the Download tab opens, then
/// fill the distro list.
fn load_catalog(ui: &Rc<Ui>) {
    if *ui.catalog_loaded.borrow() {
        return;
    }
    *ui.catalog_loaded.borrow_mut() = true;
    ui.download_stack.set_visible_child_name("loading");

    let (tx, rx) = async_channel::bounded::<Vec<catalog::Distro>>(1);
    thread::spawn(move || {
        let _ = tx.send_blocking(catalog::load());
    });

    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let Ok(distros) = rx.recv().await else {
            return;
        };
        if distros.is_empty() {
            ui.download_stack.set_visible_child_name("empty");
        } else {
            populate_catalog(&ui, distros);
            ui.download_stack.set_visible_child_name("ready");
        }
    });
}

/// Fill the distro list. Each distro's image rows are built the first time it is
/// opened, so a thousand-image catalogue stays cheap to show.
fn populate_catalog(ui: &Rc<Ui>, distros: Vec<catalog::Distro>) {
    for distro in distros {
        let count = distro.images.len();
        let exp = adw::ExpanderRow::builder()
            .title(&distro.name)
            .subtitle(format!("{count} image{}", if count == 1 { "" } else { "s" }))
            .build();

        let images = Rc::new(distro.images);
        let filled = Rc::new(Cell::new(false));
        let ui_row = ui.clone();
        exp.connect_expanded_notify(move |exp| {
            if !exp.is_expanded() || filled.get() {
                return;
            }
            filled.set(true);
            for img in images.iter() {
                let row = adw::ActionRow::builder()
                    .title(image_title(img))
                    .subtitle(image_subtitle(img))
                    .activatable(true)
                    .build();
                row.add_suffix(&gtk::Image::from_icon_name("folder-download-symbolic"));
                {
                    let ui = ui_row.clone();
                    let img = img.clone();
                    row.connect_activated(move |_| start_download(&ui, &img));
                }
                exp.add_row(&row);
            }
        });
        ui.download_list.append(&exp);
    }
}

fn image_title(img: &catalog::Image) -> String {
    match &img.variant {
        Some(v) => format!("{} · {}", img.os_name, v),
        None => img.os_name.clone(),
    }
}

fn image_subtitle(img: &catalog::Image) -> String {
    let kind = if img.live { "live" } else { "installer" };
    format!("{} · {}", img.arch, kind)
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
        ui.toast("Choose an image first.");
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

    /// Paint the hero/badge for the current state.
    fn apply_state_style(&self, kind: &str, icon: &str) {
        let hero = if kind == "idle" || kind == "off" {
            vec!["bd-hero".to_string()]
        } else {
            vec!["bd-hero".to_string(), kind.to_string()]
        };
        self.hero.set_css_classes(&hero.iter().map(String::as_str).collect::<Vec<_>>());
        let badge = if kind == "idle" || kind == "off" {
            vec!["bd-badge"]
        } else {
            vec!["bd-badge", kind]
        };
        self.badge.set_css_classes(&badge);
        self.badge_icon.set_icon_name(Some(icon));
    }

    fn refresh(&self) {
        let state = *self.state.borrow();
        let available = *self.available.borrow();

        if !available {
            self.mount_stack.set_visible_child_name("setup");
            return;
        }
        self.mount_stack.set_visible_child_name("ready");

        self.status_title.set_text(state.headline());

        let active = *self.active.borrow();
        let entry = active.and_then(|i| self.library.borrow().entries.get(i).cloned());

        self.status_detail
            .set_text(&detail_text(state, entry.as_ref()));

        let (kind, icon) = match state {
            DriveState::Preparing | DriveState::Ejecting => {
                ("busy", "content-loading-symbolic")
            }
            DriveState::Active => ("active", "object-select-symbolic"),
            DriveState::Error => ("error", "dialog-warning-symbolic"),
            _ => ("idle", "drive-removable-media-symbolic"),
        };
        self.apply_state_style(kind, icon);

        match entry {
            Some(entry) => {
                self.image_row.set_title(&entry.display_name);
                let size = entry
                    .size
                    .map(human_size)
                    .unwrap_or_else(|| "unknown size".to_string());
                self.image_row
                    .set_subtitle(&format!("{} · {}", size, entry.mode().label()));
                self.mode_row.set_visible(entry.hybrid);
                self.mode_row.set_selected(if entry.cdrom { 0 } else { 1 });
            }
            None => {
                self.image_row.set_title("Choose an image");
                self.image_row.set_subtitle("Pick one from your library");
                self.mode_row.set_visible(false);
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

        self.mode_row
            .set_sensitive(enabled && state != DriveState::Active);

        match state {
            DriveState::Active => {
                self.primary_button.set_label("Eject");
                self.primary_button
                    .set_css_classes(&["destructive-action", "pill"]);
                self.primary_button.set_sensitive(enabled);
            }
            _ if !has_active => {
                self.primary_button.set_label("Choose an image");
                self.primary_button
                    .set_css_classes(&["suggested-action", "pill"]);
                self.primary_button.set_sensitive(enabled);
            }
            _ => {
                self.primary_button.set_label("Expose over USB");
                self.primary_button
                    .set_css_classes(&["suggested-action", "pill"]);
                self.primary_button.set_sensitive(enabled && !transitioning);
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
                "Choose an image to get started.".to_string()
            }
        }
    }
}
