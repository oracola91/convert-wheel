//! The menu bar icon and its menu: how the last conversion went, updates, Open at
//! Login, where converted files go, and which formats the wheel offers.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, MenuItemKind, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{App, AppHandle, Manager, Wry};

use crate::settings::{self, SaveTo, Store};
use crate::{formats, login, reveal, update};

const SAVE_NEXT: &str = "save:next";
const SAVE_DOWNLOADS: &str = "save:downloads";
const LOGIN: &str = "login";
const LAST: &str = "last";
const UPDATE: &str = "update";
const SHOW_LOG: &str = "log";
/// Longest "Last: …" line, so the menu stays a sensible width.
const LAST_MAX_CHARS: usize = 70;
const QUIT: &str = "quit";
/// Format switches are "fmt:" followed by the settings key, e.g. "fmt:image:avif".
const FORMAT: &str = "fmt:";

pub struct TrayMenu(Menu<Wry>);

pub fn build(app: &App) -> tauri::Result<()> {
    let current = app.state::<Store>().get();
    let version = format!("Convert Wheel {}", app.package_info().version);
    let title = MenuItem::with_id(app, "version", version, false, None::<&str>)?;
    let hint = MenuItem::with_id(app, "hint", "Hold Shift while dragging a file", false, None::<&str>)?;
    // Feedback that stays put even when notifications are off. Once a drop produced
    // something, a click on it shows the result.
    let last = MenuItem::with_id(app, LAST, "Last: nothing converted yet", false, None::<&str>)?;
    let update = MenuItem::with_id(app, UPDATE, "Check for Updates…", true, None::<&str>)?;
    let show_log = MenuItem::with_id(app, SHOW_LOG, "Show Log", true, None::<&str>)?;

    let login_label = if login::available() { "Open at Login" } else { "Open at Login (installed app only)" };
    let login = CheckMenuItem::with_id(app, LOGIN, login_label, login::available(), login::is_enabled(), None::<&str>)?;

    let next = CheckMenuItem::with_id(app, SAVE_NEXT, "Next to the Original", true, current.save_to == SaveTo::NextToOriginal, None::<&str>)?;
    let downloads = CheckMenuItem::with_id(app, SAVE_DOWNLOADS, "In Downloads", true, current.save_to == SaveTo::Downloads, None::<&str>)?;
    let save = Submenu::with_items(app, "Save Converted Files", true, &[&next, &downloads])?;

    let mut kinds = Vec::new();
    for (kind, outputs) in formats::all() {
        let items = outputs
            .iter()
            .map(|f| {
                let key = settings::key(kind, f);
                CheckMenuItem::with_id(app, format!("{FORMAT}{key}"), f.to_uppercase(), true, current.shows(kind, f), None::<&str>)
            })
            .collect::<tauri::Result<Vec<_>>>()?;
        let refs: Vec<&dyn tauri::menu::IsMenuItem<Wry>> = items.iter().map(|i| i as _).collect();
        kinds.push(Submenu::with_items(app, kind.label(), true, &refs)?);
    }
    let kind_refs: Vec<&dyn tauri::menu::IsMenuItem<Wry>> = kinds.iter().map(|k| k as _).collect();
    let shown = Submenu::with_items(app, "Formats on the Wheel", true, &kind_refs)?;

    let quit = MenuItem::with_id(app, QUIT, "Quit Convert Wheel", true, Some("CmdOrCtrl+Q"))?;
    let sep = || PredefinedMenuItem::separator(app);
    let menu = Menu::with_items(
        app,
        &[&title, &hint, &last, &sep()?, &update, &login, &save, &shown, &sep()?, &show_log, &quit],
    )?;

    // macOS: a template icon it tints to match the menu bar, light or dark. Windows'
    // tray can be light or dark too, so it gets the coloured app icon instead.
    #[cfg(target_os = "macos")]
    let tray = TrayIconBuilder::with_id("main").icon(tauri::include_image!("./icons/tray.png")).icon_as_template(true);
    #[cfg(not(target_os = "macos"))]
    let tray = match app.default_window_icon() {
        Some(icon) => TrayIconBuilder::with_id("main").icon(icon.clone()),
        None => TrayIconBuilder::with_id("main"),
    };
    tray.tooltip("Convert Wheel: hold Shift while dragging a file")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(on_event)
        .build(app)?;
    app.manage(TrayMenu(menu));
    Ok(())
}

/// Puts how the last drop went in the menu, for example "Last: Converted to PDF:
/// report.pdf", cut short to keep the menu narrow. `clickable`: the drop produced
/// files, and a click on the line shows them.
pub fn show_last_result(app: &AppHandle, text: &str, clickable: bool) {
    let mut line = format!("Last: {text}");
    if line.chars().count() > LAST_MAX_CHARS {
        line = line.chars().take(LAST_MAX_CHARS - 1).collect::<String>() + "…";
    }
    set_text(app, LAST, &line, clickable);
}

pub enum UpdateState<'a> {
    UpToDate,
    CheckFailed,
    Available(&'a str),
    Installing,
}

/// The update item says what the last check found, so it is clear without a
/// notification.
pub fn show_update(app: &AppHandle, state: UpdateState) {
    let (text, enabled) = match state {
        UpdateState::UpToDate => ("Check for Updates… (up to date)".to_string(), true),
        UpdateState::CheckFailed => ("Check for Updates… (last check failed)".to_string(), true),
        UpdateState::Available(v) => (format!("Install Update {v} and Restart"), true),
        UpdateState::Installing => ("Installing Update…".to_string(), false),
    };
    set_text(app, UPDATE, &text, enabled);
}

fn set_text(app: &AppHandle, id: &str, text: &str, enabled: bool) {
    let Some(menu) = app.try_state::<TrayMenu>() else {
        return;
    };
    if let Some(item) = find(menu.0.items().unwrap_or_default(), id) {
        if let Some(item) = item.as_menuitem() {
            let _ = item.set_text(text);
            let _ = item.set_enabled(enabled);
        }
    }
}

/// Opens the folder with the log file, the file selected where the system allows it.
fn show_log(app: &AppHandle) {
    let Some(file) = crate::log::path() else {
        crate::notify(app, "No log yet", "Convert Wheel hasn't written a log on this computer.");
        return;
    };
    reveal::show(app, vec![file]);
}

/// Quitting while conversions run asks for a second Quit within 10 seconds.
fn quit(app: &AppHandle) {
    static ASKED_AT: AtomicU64 = AtomicU64::new(0);
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let jobs = crate::busy_jobs();
    if jobs > 0 && now.saturating_sub(ASKED_AT.swap(now, Ordering::SeqCst)) > 10 {
        crate::notify(
            app,
            &format!("{jobs} conversion(s) still running"),
            "Choose Quit again to stop them and quit.",
        );
        return;
    }
    if jobs > 0 {
        crate::log::line(&format!("quit with {jobs} conversion(s) unfinished"));
    }
    app.exit(0);
}

fn set_check(app: &AppHandle, id: &str, checked: bool) {
    let menu = &app.state::<TrayMenu>().0;
    if let Some(item) = find(menu.items().unwrap_or_default(), id) {
        if let Some(check) = item.as_check_menuitem() {
            let _ = check.set_checked(checked);
        }
    }
}

/// Finds a menu item by id, also inside submenus.
fn find(items: Vec<MenuItemKind<Wry>>, id: &str) -> Option<MenuItemKind<Wry>> {
    for item in items {
        if item.id() == id {
            return Some(item);
        }
        if let Some(sub) = item.as_submenu() {
            if let Some(found) = find(sub.items().unwrap_or_default(), id) {
                return Some(found);
            }
        }
    }
    None
}

fn on_event(app: &AppHandle, event: MenuEvent) {
    let id = event.id().as_ref();
    let store = app.state::<Store>();
    match id {
        QUIT => quit(app),
        SHOW_LOG => show_log(app),
        LAST => reveal::show_last(app),
        UPDATE => {
            if update::pending(app).is_some() {
                update::install_soon(app);
            } else {
                update::check_soon(app);
            }
        }
        LOGIN => {
            if let Err(e) = login::set_enabled(!login::is_enabled()) {
                crate::notify(app, "Couldn't change Open at Login", &e);
            }
            set_check(app, LOGIN, login::is_enabled());
        }
        SAVE_NEXT | SAVE_DOWNLOADS => {
            let wanted = if id == SAVE_NEXT { SaveTo::NextToOriginal } else { SaveTo::Downloads };
            if let Err(e) = store.update(|s| s.save_to = wanted) {
                crate::notify(app, "Couldn't save the setting", &e);
            }
            // Two choices, exactly one ticked, matching what is saved.
            let save_to = store.get().save_to;
            set_check(app, SAVE_NEXT, save_to == SaveTo::NextToOriginal);
            set_check(app, SAVE_DOWNLOADS, save_to == SaveTo::Downloads);
        }
        _ => {
            if let Some(key) = id.strip_prefix(FORMAT) {
                if let Err(e) = store.update(|s| {
                    if !s.hidden.remove(key) {
                        s.hidden.insert(key.to_string());
                    }
                }) {
                    crate::notify(app, "Couldn't save the setting", &e);
                }
                set_check(app, id, !store.get().hidden.contains(key));
            }
        }
    }
}
