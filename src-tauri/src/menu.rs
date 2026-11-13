//! The menu bar icon and its short menu: the version, how the last conversion went
//! (with Stop Converting while one runs), converters to install when a drop found one
//! missing, Settings…, updates and Quit. Everything else is in the Settings window
//! (settings_window.rs).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::menu::{Menu, MenuEvent, MenuItem, MenuItemKind, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{App, AppHandle, Manager, Wry};

use crate::{converters, reveal, update};

const LAST: &str = "last";
const STOP: &str = "stop";
const INSTALL: &str = "install";
const SETTINGS: &str = "settings";
const UPDATE: &str = "update";
const TOOLTIP: &str = "hold Shift while dragging a file";
/// Longest "Last: …" line, so the menu stays a sensible width.
pub const LAST_MAX_CHARS: usize = 70;
const QUIT: &str = "quit";

pub struct TrayMenu(Menu<Wry>);

/// The items that come and go: Stop Converting (while a drop converts) and Install
/// (while a drop found a converter missing). Kept here while out of the menu; the lock
/// keeps two threads from both putting one in.
struct Coming {
    stop: Mutex<MenuItem<Wry>>,
    install: Mutex<MenuItem<Wry>>,
}

pub fn build(app: &App) -> tauri::Result<()> {
    let version = format!("{} {}", crate::APP_NAME, app.package_info().version);
    let title = MenuItem::with_id(app, "version", version, false, None::<&str>)?;
    // Feedback that stays put even when notifications are off. Once a drop produced
    // something, a click on it shows the result.
    let last = MenuItem::with_id(app, LAST, "Last: nothing converted yet", false, None::<&str>)?;
    let stop = MenuItem::with_id(app, STOP, "Stop Converting", true, None::<&str>)?;
    let install = MenuItem::with_id(app, INSTALL, "Install", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, SETTINGS, "Settings…", true, Some("CmdOrCtrl+,"))?;
    let update = MenuItem::with_id(app, UPDATE, "Check for Updates…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT, format!("Quit {}", crate::APP_NAME), true, Some("CmdOrCtrl+Q"))?;
    let (sep, sep2) = (PredefinedMenuItem::separator(app)?, PredefinedMenuItem::separator(app)?);
    let menu = Menu::with_items(app, &[&title, &last, &sep, &settings, &update, &sep2, &quit])?;

    // macOS: a template icon it tints to match the menu bar, light or dark. Windows'
    // tray can be light or dark too, so it gets the coloured app icon instead.
    #[cfg(target_os = "macos")]
    let tray = TrayIconBuilder::with_id("main").icon(tauri::include_image!("./icons/tray.png")).icon_as_template(true);
    #[cfg(not(target_os = "macos"))]
    let tray = match app.default_window_icon() {
        Some(icon) => TrayIconBuilder::with_id("main").icon(icon.clone()),
        None => TrayIconBuilder::with_id("main"),
    };
    tray.tooltip(format!("{}: {TOOLTIP}", crate::APP_NAME))
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(on_event)
        .build(app)?;
    app.manage(TrayMenu(menu));
    app.manage(Coming { stop: Mutex::new(stop), install: Mutex::new(install) });
    Ok(())
}

/// Shows or hides the menu bar (tray) icon, from Settings > General. Hidden, the app
/// still runs; opening it again shows Settings.
pub fn show_icon(app: &AppHandle, shown: bool) {
    if let Some(tray) = app.tray_by_id("main") {
        if let Err(e) = tray.set_visible(shown) {
            crate::log::line(&format!("couldn't {} the menu bar icon: {e}", if shown { "show" } else { "hide" }));
        }
    }
}

/// Puts how the last drop went in the menu, for example "Last: Converted to PDF:
/// report.pdf", cut short to keep the menu narrow. `clickable`: the drop produced
/// files, and a click on the line shows them. Stop Converting goes away.
pub fn show_last_result(app: &AppHandle, text: &str, clickable: bool) {
    set_text(app, LAST, &short(&format!("Last: {text}")), clickable);
    show_coming(app, STOP, None, true);
    set_tooltip(app, &format!("{}: {TOOLTIP}", crate::APP_NAME));
}

/// While a drop converts: how far it has got, in the menu and the menu bar icon's
/// tooltip, with Stop Converting under it.
pub fn show_progress(app: &AppHandle, line: &str) {
    set_text(app, LAST, &short(line), false);
    show_coming(app, STOP, Some("Stop Converting"), true);
    set_tooltip(app, &format!("{}: {line}", crate::APP_NAME));
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
    show_coming(app, INSTALL, text, enabled);
}

/// Puts Stop Converting or Install in the menu under the Last line (Install under
/// Stop when both are there) with `text`, or with None takes it out.
fn show_coming(app: &AppHandle, id: &str, text: Option<&str>, enabled: bool) {
    let (Some(menu), Some(coming)) = (app.try_state::<TrayMenu>(), app.try_state::<Coming>()) else {
        return;
    };
    let item = if id == STOP { &coming.stop } else { &coming.install };
    let item = item.lock().unwrap_or_else(|e| e.into_inner());
    let items = menu.0.items().unwrap_or_default();
    let shown = items.iter().any(|i| i.id() == id);
    match text {
        Some(text) => {
            let _ = item.set_text(text);
            let _ = item.set_enabled(enabled);
            if !shown {
                let _ = menu.0.insert(&*item, place_for(&items, id));
            }
        }
        None if shown => {
            let _ = menu.0.remove(&*item);
        }
        None => {}
    }
}

/// Where a coming item goes: right under the Last line, and Install under Stop.
fn place_for(items: &[MenuItemKind<Wry>], id: &str) -> usize {
    let ids: Vec<String> = items.iter().map(|i| i.id().as_ref().to_string()).collect();
    slot(&ids, id)
}

/// `place_for` on plain ids, so it can be tested without a menu.
fn slot(ids: &[String], id: &str) -> usize {
    let last = ids.iter().position(|i| i == LAST).map_or(ids.len(), |p| p + 1);
    if id == INSTALL && ids.get(last).is_some_and(|i| i == STOP) {
        last + 1
    } else {
        last
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
    if let Some(item) = menu.0.items().unwrap_or_default().into_iter().find(|i| i.id() == id) {
        if let Some(item) = item.as_menuitem() {
            let _ = item.set_text(text);
            let _ = item.set_enabled(enabled);
        }
    }
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

fn on_event(app: &AppHandle, event: MenuEvent) {
    match event.id().as_ref() {
        QUIT => quit(app),
        LAST => reveal::show_last(app),
        STOP => crate::stop_converting(app),
        INSTALL => converters::install(app),
        SETTINGS => crate::settings_window::open(app),
        UPDATE => {
            if update::pending(app).is_some() {
                update::install_soon(app);
            } else {
                update::check_soon(app);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn stop_and_install_go_under_the_last_line() {
        let short = ids(&["version", LAST, "sep", SETTINGS, UPDATE, "sep2", QUIT]);
        assert_eq!(slot(&short, STOP), 2);
        assert_eq!(slot(&short, INSTALL), 2);
        let converting = ids(&["version", LAST, STOP, "sep", SETTINGS, UPDATE, "sep2", QUIT]);
        assert_eq!(slot(&converting, INSTALL), 3);
        let installing = ids(&["version", LAST, INSTALL, "sep", SETTINGS, UPDATE, "sep2", QUIT]);
        assert_eq!(slot(&installing, STOP), 2);
    }
}
