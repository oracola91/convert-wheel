//! The menu bar icon and its menu: Open at Login, where converted files go, and which
//! formats the wheel offers.

use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, MenuItemKind, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{App, AppHandle, Manager, Wry};

use crate::formats;
use crate::login;
use crate::settings::{self, SaveTo, Store};

const SAVE_NEXT: &str = "save:next";
const SAVE_DOWNLOADS: &str = "save:downloads";
const LOGIN: &str = "login";
const QUIT: &str = "quit";
/// Format switches are "fmt:" followed by the settings key, e.g. "fmt:image:avif".
const FORMAT: &str = "fmt:";

pub struct TrayMenu(Menu<Wry>);

pub fn build(app: &App) -> tauri::Result<()> {
    let current = app.state::<Store>().get();
    let version = format!("Convert Wheel {}", app.package_info().version);
    let title = MenuItem::with_id(app, "version", version, false, None::<&str>)?;
    let hint = MenuItem::with_id(app, "hint", "Hold Shift while dragging a file", false, None::<&str>)?;

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
    let menu = Menu::with_items(app, &[&title, &hint, &sep()?, &login, &save, &shown, &sep()?, &quit])?;

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
        QUIT => app.exit(0),
        LOGIN => {
            if let Err(e) = login::set_enabled(!login::is_enabled()) {
                crate::notify(app, "Couldn't change Open at Login", &e);
            }
            set_check(app, LOGIN, login::is_enabled());
        }
        SAVE_NEXT | SAVE_DOWNLOADS => {
            let save_to = if id == SAVE_NEXT { SaveTo::NextToOriginal } else { SaveTo::Downloads };
            store.update(|s| s.save_to = save_to);
            // Two choices, exactly one ticked.
            set_check(app, SAVE_NEXT, save_to == SaveTo::NextToOriginal);
            set_check(app, SAVE_DOWNLOADS, save_to == SaveTo::Downloads);
        }
        _ => {
            if let Some(key) = id.strip_prefix(FORMAT) {
                let now = store.update(|s| {
                    if !s.hidden.remove(key) {
                        s.hidden.insert(key.to_string());
                    }
                });
                set_check(app, id, !now.hidden.contains(key));
            }
        }
    }
}
