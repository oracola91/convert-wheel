//! The shelf on Windows and Linux (X11): drag files to the top edge of the screen and
//! a small shelf opens at the top centre to hold them; drag them out of it later, into
//! any folder or app. As on macOS (shelf_window.rs), holding a file only remembers where
//! it is (shelf.rs), and files that landed somewhere leave the shelf. While it holds
//! files a pill stays at the top centre; pointing at it opens the shelf.
//!
//! The window is a small page, shelf.html. Files come in through the system's drop
//! (Windows: win32.rs's drop target; Linux: the window's own drop event) and go out as
//! Float's cards do (float_web.rs). Wayland lets no app place a window at the top of
//! the screen, so there is no shelf there.

#![cfg_attr(target_os = "macos", allow(dead_code))]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::log;
use crate::shelf::Shelf;

pub const LABEL: &str = "shelf";
const OPEN: (f64, f64) = (320.0, 120.0);
const PILL: (f64, f64) = (170.0, 34.0);
/// How close to the top edge of the screen a drag opens the shelf, in pixels.
pub const EDGE: i32 = 3;

/// A drag is over the top of the screen right now, so the shelf stays open.
static DRAGGING: AtomicBool = AtomicBool::new(false);

#[derive(Serialize)]
pub struct Held {
    name: String,
    path: String,
    uri: String,
}

#[derive(Serialize)]
pub struct Shown {
    items: Vec<Held>,
    /// Windows: the page asks for the system's drag instead of starting its own.
    native: bool,
    open: bool,
}

fn wayland() -> bool {
    #[cfg(target_os = "linux")]
    return crate::linux::is_wayland();
    #[allow(unreachable_code)]
    false
}

fn on(app: &AppHandle) -> bool {
    !app.state::<crate::settings::Store>().get().shelf_off && !wayland()
}

/// At startup: the pill, if the shelf still holds files from last time.
pub fn start(app: &AppHandle) {
    if !on(app) {
        return;
    }
    let shelf = app.state::<Shelf>();
    shelf.prune();
    if !shelf.is_empty() {
        show(app, false);
    }
}

/// A file drag reached the top edge of the screen: open the shelf to take it.
pub fn drag_near_top(app: &AppHandle) {
    if on(app) && !DRAGGING.swap(true, Ordering::SeqCst) {
        show(app, true);
    }
}

/// The drag ended (dropped anywhere, or cancelled): back to the pill, or away.
pub fn drag_ended(app: &AppHandle) {
    if DRAGGING.swap(false, Ordering::SeqCst) {
        settle(app);
    }
}

/// The pill while files are held, nothing otherwise.
fn settle(app: &AppHandle) {
    if app.state::<Shelf>().is_empty() || !on(app) {
        if let Some(w) = app.get_webview_window(LABEL) {
            let _ = w.hide();
        }
    } else {
        show(app, false);
    }
}

/// The menu's Shelf at the Top of the Screen was ticked or unticked.
pub fn setting_changed(app: &AppHandle) {
    settle(app);
}

/// Files were dropped on the shelf.
pub fn dropped(app: &AppHandle, paths: Vec<String>) {
    let added = app.state::<Shelf>().add(paths);
    log::line(&format!("shelf: {added} file(s) held"));
    changed(app);
}

fn changed(app: &AppHandle) {
    let _ = app.emit_to(LABEL, "shelf-changed", ());
    if !DRAGGING.load(Ordering::SeqCst) {
        settle(app);
    }
}

/// Opens (or makes) the window at the top centre of the screen the pointer is on.
fn show(app: &AppHandle, open: bool) {
    let (w, h) = if open { OPEN } else { PILL };
    let (x, y) = top_centre(app, w);
    let window = match app.get_webview_window(LABEL) {
        Some(window) => window,
        None => match WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("shelf.html".into()))
            .title("Shelf")
            .inner_size(w, h)
            .position(x, y)
            .resizable(false)
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible_on_all_workspaces(true)
            .focused(false)
            .visible(false)
            .build()
        {
            Ok(window) => {
                #[cfg(target_os = "linux")]
                linux_drop(app, &window);
                window
            }
            Err(e) => {
                log::line(&format!("couldn't open the shelf: {e}"));
                return;
            }
        },
    };
    let _ = window.set_size(tauri::LogicalSize::new(w, h));
    let _ = window.set_position(tauri::LogicalPosition::new(x, y));
    let _ = window.show();
    // OLE drop targets belong to the window's own thread: the main one.
    #[cfg(windows)]
    {
        let (a, w) = (app.clone(), window.clone());
        let _ = app.run_on_main_thread(move || crate::win32::register_shelf_drop(&a, &w));
    }
    let _ = window.emit_to(LABEL, "shelf-changed", ());
}

/// Linux: the window's own drop event carries the files' paths.
#[cfg(target_os = "linux")]
fn linux_drop(app: &AppHandle, window: &WebviewWindow) {
    let app = app.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) = event {
            let paths: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
            if !paths.is_empty() {
                dropped(&app, paths);
            }
        }
    });
}

fn top_centre(app: &AppHandle, width: f64) -> (f64, f64) {
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(m) = monitor else {
        return (400.0, 4.0);
    };
    let scale = m.scale_factor();
    let area = m.work_area();
    let left = area.position.x as f64 / scale;
    let top = area.position.y as f64 / scale;
    let span = area.size.width as f64 / scale;
    (left + (span - width) / 2.0, top + 4.0)
}

fn items(app: &AppHandle) -> Vec<PathBuf> {
    app.state::<Shelf>().held().into_iter().map(PathBuf::from).collect()
}

/// The page asks what the shelf holds, newest first.
#[tauri::command]
pub fn shelf_items(app: AppHandle) -> Shown {
    let mut held: Vec<Held> = items(&app)
        .iter()
        .map(|p| Held {
            name: p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string()),
            path: p.display().to_string(),
            uri: crate::float_web::file_uri(p),
        })
        .collect();
    held.reverse();
    let open = app.get_webview_window(LABEL).and_then(|w| w.inner_size().ok()).is_some_and(|s| s.height > 60);
    Shown { items: held, native: cfg!(windows), open }
}

/// The pointer came onto the pill (open) or left the open shelf (pill).
#[tauri::command]
pub fn shelf_open(app: AppHandle, open: bool) {
    if open {
        show(&app, true);
    } else if !DRAGGING.load(Ordering::SeqCst) {
        settle(&app);
    }
}

/// Windows: the system's drag of every held file. Those that landed leave the shelf.
#[tauri::command]
pub fn shelf_drag(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    let files = items(&app);
    if crate::float_web::drag_out(&window, &files)? {
        let gone: Vec<String> = files.iter().map(|f| f.display().to_string()).collect();
        app.state::<Shelf>().remove(&gone);
    }
    changed(&app);
    Ok(())
}

/// Linux: the page's own drag of the held files ended; `landed` unless cancelled.
#[tauri::command]
pub fn shelf_dragged(app: AppHandle, landed: bool) {
    if landed {
        let gone: Vec<String> = items(&app).iter().map(|f| f.display().to_string()).collect();
        app.state::<Shelf>().remove(&gone);
    } else {
        app.state::<Shelf>().prune();
    }
    changed(&app);
}

/// The × lets go of everything. Nothing on disk changes.
#[tauri::command]
pub fn shelf_clear(app: AppHandle) {
    app.state::<Shelf>().clear();
    changed(&app);
}

/// A click on a file shows it in the file manager.
#[tauri::command]
pub fn shelf_reveal(app: AppHandle, path: String) {
    if items(&app).iter().any(|p| p.display().to_string() == path) {
        crate::reveal::show(&app, vec![PathBuf::from(path)]);
    }
}
