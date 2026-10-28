//! The menu bar icon and its menu: how the last conversion went, converters to
//! install, updates, Open at Login, the shelf (macOS), where converted files go, which
//! formats the wheel offers, and the quality per kind of file.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, MenuItemKind, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{App, AppHandle, Manager, Wry};

use crate::settings::{self, SaveTo, Store};
use crate::{converters, formats, login, reveal, update};

const SAVE_NEXT: &str = "save:next";
const SAVE_DOWNLOADS: &str = "save:downloads";
const LOGIN: &str = "login";
#[cfg(target_os = "macos")]
const SHELF: &str = "shelf";
const LAST: &str = "last";
const STOP: &str = "stop";
const INSTALL: &str = "install";
/// Where the Install item goes while converters are missing: under Stop Converting.
const INSTALL_AT: usize = 4;
const UPDATE: &str = "update";
const TOOLTIP: &str = "Convert Wheel: hold Shift while dragging a file";
const SHOW_LOG: &str = "log";
/// Longest "Last: …" line, so the menu stays a sensible width.
pub const LAST_MAX_CHARS: usize = 70;
const QUIT: &str = "quit";
/// Format switches are "fmt:" followed by the settings key, e.g. "fmt:image:avif".
const FORMAT: &str = "fmt:";
/// Quality choices are "quality:" followed by the kind and the level, e.g.
/// "quality:video:best".
const QUALITY: &str = "quality:";
/// Highlight colours are "color:" followed by the colour, e.g. "color:blue".
const COLOR: &str = "color:";
/// Wheel glass choices are "glass:" followed by the kind, e.g. "glass:clear" (macOS).
#[cfg(target_os = "macos")]
const GLASS: &str = "glass:";

pub struct TrayMenu(Menu<Wry>);

/// The Install item, kept while it is out of the menu. The lock keeps two threads from
/// both putting it in.
struct InstallItem(Mutex<MenuItem<Wry>>);

pub fn build(app: &App) -> tauri::Result<()> {
    let current = app.state::<Store>().get();
    let version = format!("Convert Wheel {}", app.package_info().version);
    let title = MenuItem::with_id(app, "version", version, false, None::<&str>)?;
    let hint = MenuItem::with_id(app, "hint", "Hold Shift while dragging a file", false, None::<&str>)?;
    // Feedback that stays put even when notifications are off. Once a drop produced
    // something, a click on it shows the result.
    let last = MenuItem::with_id(app, LAST, "Last: nothing converted yet", false, None::<&str>)?;
    let stop = MenuItem::with_id(app, STOP, "Stop Converting", false, None::<&str>)?;
    // Only in the menu once a drop found a converter missing; see show_install.
    let install = MenuItem::with_id(app, INSTALL, "Install", true, None::<&str>)?;
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

    let mut levels = Vec::new();
    for (kind, label) in settings::Quality::KINDS {
        let items = settings::Level::ALL
            .iter()
            .map(|level| {
                let id = format!("{QUALITY}{kind}:{}", level.key());
                CheckMenuItem::with_id(app, id, level.label(), true, current.quality.get(kind) == Some(*level), None::<&str>)
            })
            .collect::<tauri::Result<Vec<_>>>()?;
        let refs: Vec<&dyn tauri::menu::IsMenuItem<Wry>> = items.iter().map(|i| i as _).collect();
        levels.push(Submenu::with_items(app, label, true, &refs)?);
    }
    let level_refs: Vec<&dyn tauri::menu::IsMenuItem<Wry>> = levels.iter().map(|k| k as _).collect();
    let quality = Submenu::with_items(app, "Quality", true, &level_refs)?;

    let colors = settings::Highlight::ALL
        .iter()
        .map(|h| CheckMenuItem::with_id(app, format!("{COLOR}{}", h.key()), h.label(), true, current.highlight == *h, None::<&str>))
        .collect::<tauri::Result<Vec<_>>>()?;
    let color_refs: Vec<&dyn tauri::menu::IsMenuItem<Wry>> = colors.iter().map(|c| c as _).collect();
    let highlight = Submenu::with_items(app, "Highlight Color", true, &color_refs)?;

    let quit = MenuItem::with_id(app, QUIT, "Quit Convert Wheel", true, Some("CmdOrCtrl+Q"))?;
    let sep = || PredefinedMenuItem::separator(app);
    let (separator, separator2) = (sep()?, sep()?);
    let mut items: Vec<&dyn tauri::menu::IsMenuItem<Wry>> = vec![&title, &hint, &last, &stop, &separator, &update, &login];
    // Only macOS has the shelf so far.
    #[cfg(target_os = "macos")]
    let shelf = CheckMenuItem::with_id(app, SHELF, "Shelf at the Top of the Screen", true, !current.shelf_off, None::<&str>)?;
    #[cfg(target_os = "macos")]
    items.push(&shelf);
    let middle: [&dyn tauri::menu::IsMenuItem<Wry>; 4] = [&save, &shown, &quality, &highlight];
    items.extend(middle);
    // Only macOS has a choice of glass.
    #[cfg(target_os = "macos")]
    let kinds = settings::Glass::ALL
        .iter()
        .map(|g| CheckMenuItem::with_id(app, format!("{GLASS}{}", g.key()), g.label(), true, current.glass == *g, None::<&str>))
        .collect::<tauri::Result<Vec<_>>>()?;
    #[cfg(target_os = "macos")]
    let kind_items: Vec<&dyn tauri::menu::IsMenuItem<Wry>> = kinds.iter().map(|k| k as _).collect();
    #[cfg(target_os = "macos")]
    let glass = Submenu::with_items(app, "Wheel Glass", true, &kind_items)?;
    #[cfg(target_os = "macos")]
    items.push(&glass);
    let rest: [&dyn tauri::menu::IsMenuItem<Wry>; 3] = [&separator2, &show_log, &quit];
    items.extend(rest);
    let menu = Menu::with_items(app, &items)?;

    // macOS: a template icon it tints to match the menu bar, light or dark. Windows'
    // tray can be light or dark too, so it gets the coloured app icon instead.
    #[cfg(target_os = "macos")]
    let tray = TrayIconBuilder::with_id("main").icon(tauri::include_image!("./icons/tray.png")).icon_as_template(true);
    #[cfg(not(target_os = "macos"))]
    let tray = match app.default_window_icon() {
        Some(icon) => TrayIconBuilder::with_id("main").icon(icon.clone()),
        None => TrayIconBuilder::with_id("main"),
    };
    tray.tooltip(TOOLTIP)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(on_event)
        .build(app)?;
    app.manage(TrayMenu(menu));
    app.manage(InstallItem(Mutex::new(install)));
    Ok(())
}

/// Puts how the last drop went in the menu, for example "Last: Converted to PDF:
/// report.pdf", cut short to keep the menu narrow. `clickable`: the drop produced
/// files, and a click on the line shows them.
pub fn show_last_result(app: &AppHandle, text: &str, clickable: bool) {
    set_text(app, LAST, &short(&format!("Last: {text}")), clickable);
    set_text(app, STOP, "Stop Converting", false);
    set_tooltip(app, TOOLTIP);
}

/// While a drop converts: how far it has got, in the menu and the menu bar icon's
/// tooltip, and Stop Converting can be chosen.
pub fn show_progress(app: &AppHandle, line: &str) {
    set_text(app, LAST, &short(line), false);
    set_text(app, STOP, "Stop Converting", true);
    set_tooltip(app, &format!("Convert Wheel: {line}"));
}

fn short(line: &str) -> String {
    if line.chars().count() > LAST_MAX_CHARS {
        line.chars().take(LAST_MAX_CHARS - 1).collect::<String>() + "…"
    } else {
        line.to_string()
    }
}

/// Linux trays show no tooltip; there this does nothing.
fn set_tooltip(app: &AppHandle, text: &str) {
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(text));
    }
}

/// Offers to install the converters drops found missing (`text` such as "Install
/// ffmpeg…"), or with None takes the item out of the menu again.
pub fn show_install(app: &AppHandle, text: Option<&str>, enabled: bool) {
    let (Some(menu), Some(install)) = (app.try_state::<TrayMenu>(), app.try_state::<InstallItem>()) else {
        return;
    };
    let item = install.0.lock().unwrap_or_else(|e| e.into_inner());
    let shown = find(menu.0.items().unwrap_or_default(), INSTALL).is_some();
    match text {
        Some(text) => {
            let _ = item.set_text(text);
            let _ = item.set_enabled(enabled);
            if !shown {
                let _ = menu.0.insert(&*item, INSTALL_AT);
            }
        }
        None if shown => {
            let _ = menu.0.remove(&*item);
        }
        None => {}
    }
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
        STOP => crate::stop_converting(app),
        INSTALL => converters::install(app),
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
        #[cfg(target_os = "macos")]
        SHELF => {
            if let Err(e) = store.update(|s| s.shelf_off = !s.shelf_off) {
                crate::notify(app, "Couldn't save the setting", &e);
            }
            set_check(app, SHELF, !store.get().shelf_off);
            crate::shelf_window::setting_changed(app);
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
        #[cfg(target_os = "macos")]
        _ if id.starts_with(GLASS) => {
            let wanted = &id[GLASS.len()..];
            let Some(kind) = settings::Glass::ALL.into_iter().find(|g| g.key() == wanted) else {
                return;
            };
            if let Err(e) = store.update(|s| s.glass = kind) {
                crate::notify(app, "Couldn't save the setting", &e);
            }
            // One kind ticked, the one saved; the next wheel opens with it.
            let saved = store.get().glass;
            for g in settings::Glass::ALL {
                set_check(app, &format!("{GLASS}{}", g.key()), saved == g);
            }
        }
        _ if id.starts_with(COLOR) => {
            let wanted = &id[COLOR.len()..];
            let Some(color) = settings::Highlight::ALL.into_iter().find(|h| h.key() == wanted) else {
                return;
            };
            if let Err(e) = store.update(|s| s.highlight = color) {
                crate::notify(app, "Couldn't save the setting", &e);
            }
            // One colour ticked, the one saved; the next wheel opens in it.
            let saved = store.get().highlight;
            for h in settings::Highlight::ALL {
                set_check(app, &format!("{COLOR}{}", h.key()), saved == h);
            }
        }
        _ if id.starts_with(QUALITY) => {
            let Some((kind, wanted)) = id[QUALITY.len()..].split_once(':') else {
                return;
            };
            let Some(level) = settings::Level::ALL.into_iter().find(|l| l.key() == wanted) else {
                return;
            };
            if let Err(e) = store.update(|s| s.quality.set(kind, level)) {
                crate::notify(app, "Couldn't save the setting", &e);
            }
            // Three choices, exactly one ticked, matching what is saved.
            let saved = store.get().quality.get(kind);
            for l in settings::Level::ALL {
                set_check(app, &format!("{QUALITY}{kind}:{}", l.key()), saved == Some(l));
            }
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
