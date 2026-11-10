//! The menu bar icon and its menu: how the last conversion went, converters to
//! install, updates, Open at Login, the shelf (macOS), where converted files go, which
//! formats the wheel offers, and the quality per kind of file.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, MenuItemKind, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{App, AppHandle, Manager, Wry};

use crate::settings::{self, Combo, Keys, SaveTo, Store};
use crate::wheel::Which;
use crate::{converters, formats, login, reveal, update};

const SAVE_NEXT: &str = "save:next";
const SAVE_DOWNLOADS: &str = "save:downloads";
const LOGIN: &str = "login";
const SHELF: &str = "shelf";
/// macOS: the arrow keys on the wheel, off until switched on (Input Monitoring).
#[cfg(target_os = "macos")]
const ARROWS: &str = "arrows";
const LAST: &str = "last";
const STOP: &str = "stop";
const INSTALL: &str = "install";
/// Where the Install item goes while converters are missing: under Stop Converting.
const INSTALL_AT: usize = 4;
const UPDATE: &str = "update";
/// Newer versions of the converters, from the package manager that installed them.
const UPDATE_CONVERTERS: &str = "update-converters";
const UPDATE_CONVERTERS_LABEL: &str = "Update Converters…";
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
/// The keys for a wheel are "keys:" then the wheel then the keys, e.g.
/// "keys:tools:shift-option", or "keys:tools:off".
const KEYS: &str = "keys:";
/// The wheels in the Keys menu: as the menu ids name them, and as it shows them.
const WHEELS: [(Which, &str, &str); 3] =
    [(Which::Formats, "formats", "Formats Wheel"), (Which::Tools, "tools", "Tools Wheel"), (Which::Actions, "actions", "Actions Wheel")];
/// Wheel Glass: "glass:on" (Liquid Glass, See-through) and "glass:off" (No Liquid Glass,
/// Solid), and on Windows and Linux "glass:t50", a Transparency step.
const GLASS: &str = "glass:";
/// Windows and Linux: the Transparency steps in place of macOS's slider.
#[cfg_attr(target_os = "macos", allow(dead_code))]
const GLASS_STEPS: &[(u8, &str)] = &[(20, "Less"), (50, "Medium"), (80, "More")];

/// The step closest to a saved transparency (the macOS slider saves any value).
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn nearest_step(value: u8) -> u8 {
    GLASS_STEPS.iter().map(|(s, _)| *s).min_by_key(|s| s.abs_diff(value)).unwrap_or(50)
}

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
    let update_converters = MenuItem::with_id(app, UPDATE_CONVERTERS, UPDATE_CONVERTERS_LABEL, true, None::<&str>)?;
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

    // Keys: for each wheel, the keys held with Shift that open it, or Off.
    let mut wheels = Vec::new();
    for (which, key, label) in WHEELS {
        let now = current.keys.get(which);
        let mut choices = Combo::ALL
            .iter()
            .map(|c| CheckMenuItem::with_id(app, format!("{KEYS}{key}:{}", c.key()), c.label(), true, now == Some(*c), None::<&str>))
            .collect::<tauri::Result<Vec<_>>>()?;
        choices.push(CheckMenuItem::with_id(app, format!("{KEYS}{key}:off"), "Off", true, now.is_none(), None::<&str>)?);
        let refs: Vec<&dyn tauri::menu::IsMenuItem<Wry>> = choices.iter().map(|c| c as _).collect();
        wheels.push(Submenu::with_items(app, label, true, &refs)?);
    }
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut wheel_refs: Vec<&dyn tauri::menu::IsMenuItem<Wry>> = wheels.iter().map(|w| w as _).collect();
    // macOS reads the arrow keys only once they are switched on here: that needs Input
    // Monitoring, and macOS asks for it when they are. Windows and Linux always read them.
    #[cfg(target_os = "macos")]
    let (arrows_sep, arrows) = (
        PredefinedMenuItem::separator(app)?,
        CheckMenuItem::with_id(app, ARROWS, "Arrow Keys on the Wheel", true, current.arrow_keys, None::<&str>)?,
    );
    #[cfg(target_os = "macos")]
    wheel_refs.extend([&arrows_sep as &dyn tauri::menu::IsMenuItem<Wry>, &arrows]);
    let keys = Submenu::with_items(app, "Keys", true, &wheel_refs)?;

    let quit = MenuItem::with_id(app, QUIT, "Quit Convert Wheel", true, Some("CmdOrCtrl+Q"))?;
    let sep = || PredefinedMenuItem::separator(app);
    let (separator, separator2) = (sep()?, sep()?);
    let mut items: Vec<&dyn tauri::menu::IsMenuItem<Wry>> = vec![&title, &hint, &last, &stop, &separator, &update];
    // Where the menu can install converters, it can update them too.
    if converters::available() {
        items.push(&update_converters);
    }
    items.push(&login);
    // Every system with a shelf: not Wayland, which places no window at the top.
    let shelf = CheckMenuItem::with_id(app, SHELF, "Shelf at the Top of the Screen", true, !current.shelf_off, None::<&str>)?;
    #[cfg(target_os = "linux")]
    let shelf_here = !crate::linux::is_wayland();
    #[cfg(not(target_os = "linux"))]
    let shelf_here = true;
    if shelf_here {
        items.push(&shelf);
    }
    let middle: [&dyn tauri::menu::IsMenuItem<Wry>; 5] = [&save, &shown, &quality, &highlight, &keys];
    items.extend(middle);
    // Only macOS has a choice of glass.
    #[cfg(target_os = "macos")]
    let liquid = CheckMenuItem::with_id(app, format!("{GLASS}on"), "Liquid Glass", true, !current.no_liquid_glass, None::<&str>)?;
    #[cfg(target_os = "macos")]
    let plain = CheckMenuItem::with_id(app, format!("{GLASS}off"), "No Liquid Glass", true, current.no_liquid_glass, None::<&str>)?;
    // glass_slider.rs puts the slider under this line when the menu opens.
    #[cfg(target_os = "macos")]
    let slider_label = MenuItem::with_id(app, format!("{GLASS}label"), "Transparency", false, None::<&str>)?;
    #[cfg(target_os = "macos")]
    let glass_sep = sep()?;
    #[cfg(target_os = "macos")]
    let kind_items: Vec<&dyn tauri::menu::IsMenuItem<Wry>> = vec![&plain, &liquid, &glass_sep, &slider_label];
    #[cfg(target_os = "macos")]
    let glass = Submenu::with_items(app, "Wheel Glass", true, &kind_items)?;
    #[cfg(target_os = "macos")]
    items.push(&glass);
    // Windows and Linux: Solid or See-through, and three steps of transparency, as a
    // tray menu holds no slider.
    #[cfg(not(target_os = "macos"))]
    let glass_items = {
        let mut v = vec![
            CheckMenuItem::with_id(app, format!("{GLASS}off"), "Solid", true, current.no_liquid_glass, None::<&str>)?,
            CheckMenuItem::with_id(app, format!("{GLASS}on"), "See-through", true, !current.no_liquid_glass, None::<&str>)?,
        ];
        let near = nearest_step(current.transparency.0);
        for (step, name) in GLASS_STEPS {
            v.push(CheckMenuItem::with_id(app, format!("{GLASS}t{step}"), *name, true, *step == near, None::<&str>)?);
        }
        v
    };
    #[cfg(not(target_os = "macos"))]
    let (glass_sep, glass_label) = (sep()?, MenuItem::with_id(app, format!("{GLASS}label"), "Transparency", false, None::<&str>)?);
    #[cfg(not(target_os = "macos"))]
    let glass = {
        let mut kind: Vec<&dyn tauri::menu::IsMenuItem<Wry>> = vec![&glass_items[0], &glass_items[1], &glass_sep, &glass_label];
        kind.extend(glass_items[2..].iter().map(|i| i as &dyn tauri::menu::IsMenuItem<Wry>));
        Submenu::with_items(app, "Wheel Glass", true, &kind)?
    };
    #[cfg(not(target_os = "macos"))]
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

/// "Updating Converters…", greyed out, while the package manager runs; the menu item
/// again after.
pub fn show_converter_update(app: &AppHandle, running: bool) {
    let text = if running { "Updating Converters…" } else { UPDATE_CONVERTERS_LABEL };
    set_text(app, UPDATE_CONVERTERS, text, !running);
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

/// Ticks, for each wheel, the keys it has now (or Off).
fn show_keys(app: &AppHandle, keys: Keys) {
    for (which, key, _) in WHEELS {
        let now = keys.get(which);
        for c in Combo::ALL {
            set_check(app, &format!("{KEYS}{key}:{}", c.key()), now == Some(c));
        }
        set_check(app, &format!("{KEYS}{key}:off"), now.is_none());
    }
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
        UPDATE_CONVERTERS => converters::update(app),
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
        SHELF => {
            if let Err(e) = store.update(|s| s.shelf_off = !s.shelf_off) {
                crate::notify(app, "Couldn't save the setting", &e);
            }
            set_check(app, SHELF, !store.get().shelf_off);
            #[cfg(target_os = "macos")]
            crate::shelf_window::setting_changed(app);
            #[cfg(not(target_os = "macos"))]
            crate::shelf_web::setting_changed(app);
        }
        #[cfg(target_os = "macos")]
        ARROWS => {
            if let Err(e) = store.update(|s| s.arrow_keys = !s.arrow_keys) {
                crate::notify(app, "Couldn't save the setting", &e);
            }
            let on = store.get().arrow_keys;
            set_check(app, ARROWS, on);
            crate::trigger::set_mac_arrows(on);
            // Asked here and never by the wheel: macOS shows its dialog the first time,
            // after that the switch only lives in System Settings.
            if on && !crate::trigger::ask_input_monitoring() {
                crate::notify(
                    app,
                    "Arrow keys need Input Monitoring",
                    "Allow the app in System Settings > Privacy & Security > Input Monitoring. Until then the wheel can't see the arrow keys while another app is in front.",
                );
            }
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
        _ if id.starts_with(GLASS) => {
            if let Some(step) = id[GLASS.len()..].strip_prefix('t').and_then(|t| t.parse::<u8>().ok()) {
                if let Err(e) = store.update(|s| s.transparency = settings::Transparency(step)) {
                    crate::notify(app, "Couldn't save the setting", &e);
                }
                let near = nearest_step(store.get().transparency.0);
                for (s, _) in GLASS_STEPS {
                    set_check(app, &format!("{GLASS}t{s}"), *s == near);
                }
                return;
            }
            let plain = &id[GLASS.len()..] == "off";
            if let Err(e) = store.update(|s| s.no_liquid_glass = plain) {
                crate::notify(app, "Couldn't save the setting", &e);
            }
            // Two choices, one ticked, matching what is saved.
            let saved = store.get().no_liquid_glass;
            set_check(app, &format!("{GLASS}on"), !saved);
            set_check(app, &format!("{GLASS}off"), saved);
        }
        _ if id.starts_with(KEYS) => {
            let Some((wheel, wanted)) = id[KEYS.len()..].split_once(':') else {
                return;
            };
            let Some(&(which, _, _)) = WHEELS.iter().find(|w| w.1 == wheel) else {
                return;
            };
            let combo = match Combo::ALL.into_iter().find(|c| c.key() == wanted) {
                Some(combo) => Some(combo),
                None if wanted == "off" => None,
                None => return,
            };
            // Keys another wheel had swap over to it, so no two wheels share keys.
            if let Err(e) = store.update(|s| s.keys.assign(which, combo)) {
                crate::notify(app, "Couldn't save the setting", &e);
            }
            let saved = store.get().keys;
            crate::wheel::set_keys(saved);
            show_keys(app, saved);
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
