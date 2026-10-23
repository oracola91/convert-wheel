//! Watches for a file drag with Shift held and shows the wheel under the cursor.
//!
//! macOS has no "drag started" notification for other apps, so this polls three
//! things the window server shares with everyone: the drag pasteboard's change
//! count, the mouse buttons and the modifier keys. No permission prompt is needed.

use std::{thread, time::Duration};

use serde::Serialize;
use tauri::{AppHandle, Emitter, LogicalPosition, Manager};

use crate::{formats, wheel};

const POLL: Duration = Duration::from_millis(30);
/// How long the wheel stays after the button is released, so a drop on it can land.
const HIDE_DELAY: Duration = Duration::from_millis(400);

#[derive(Clone, Serialize)]
struct WheelShow {
    paths: Vec<String>,
    formats: Vec<&'static str>,
}

#[cfg(target_os = "macos")]
pub fn start(app: AppHandle) {
    thread::spawn(move || {
        let mut last_count = macos::drag_change_count();
        // The drag pasteboard changed while the button was down: a drag is under way.
        let mut dragging = false;
        let mut shown = false;
        loop {
            thread::sleep(POLL);
            let button_down = macos::left_button_down();
            let count = macos::drag_change_count();
            if count != last_count {
                last_count = count;
                dragging = button_down;
            }
            if !button_down {
                if shown {
                    let app = app.clone();
                    thread::spawn(move || {
                        thread::sleep(HIDE_DELAY);
                        hide_wheel(&app);
                    });
                }
                dragging = false;
                shown = false;
                continue;
            }
            if dragging && !shown && macos::shift_down() {
                let paths = macos::dragged_files();
                // Not a file drag (text, a tab, ...): ignore it until the next drag.
                if paths.is_empty() {
                    dragging = false;
                    continue;
                }
                show_wheel(&app, paths);
                shown = true;
            }
        }
    });
}

#[cfg(not(target_os = "macos"))]
pub fn start(_app: AppHandle) {}

#[cfg(target_os = "macos")]
fn show_wheel(app: &AppHandle, paths: Vec<String>) {
    let Some(win) = app.get_webview_window("wheel") else {
        return;
    };
    let (x, y_from_bottom) = macos::mouse_location();
    // AppKit measures y up from the bottom of the main screen, Tauri down from the top.
    let screen_height = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| m.size().height as f64 / m.scale_factor())
        .unwrap_or(0.0);
    let pos = LogicalPosition::new(
        x - wheel::SIZE / 2.0,
        screen_height - y_from_bottom - wheel::SIZE / 2.0,
    );
    let formats = formats::formats_for(&paths);
    if let Ok(mut shown) = app.state::<wheel::Shown>().0.lock() {
        *shown = formats.clone();
    }
    let _ = win.set_position(pos);
    let _ = win.emit("wheel-show", WheelShow { paths, formats });
    let _ = win.show();
}

pub fn hide_wheel(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("wheel") {
        let _ = win.hide();
    }
}

#[cfg(target_os = "macos")]
pub use macos::file_paths;

#[cfg(target_os = "macos")]
mod macos {
    use objc2::rc::Retained;
    use objc2_app_kit::{
        NSEvent, NSEventModifierFlags, NSPasteboard, NSPasteboardNameDrag, NSPasteboardTypeFileURL,
    };
    use objc2_foundation::NSURL;

    fn drag_pasteboard() -> Retained<NSPasteboard> {
        NSPasteboard::pasteboardWithName(unsafe { NSPasteboardNameDrag })
    }

    pub fn drag_change_count() -> isize {
        drag_pasteboard().changeCount()
    }

    pub fn left_button_down() -> bool {
        NSEvent::pressedMouseButtons() & 1 == 1
    }

    pub fn shift_down() -> bool {
        NSEvent::modifierFlags_class().contains(NSEventModifierFlags::Shift)
    }

    pub fn mouse_location() -> (f64, f64) {
        let p = NSEvent::mouseLocation();
        (p.x, p.y)
    }

    pub fn dragged_files() -> Vec<String> {
        file_paths(&drag_pasteboard())
    }

    /// Paths of the files on a pasteboard. Finder hands over file reference URLs
    /// (file:///.file/id=...), so each one is resolved to a normal path.
    pub fn file_paths(pb: &NSPasteboard) -> Vec<String> {
        let Some(items) = pb.pasteboardItems() else {
            return Vec::new();
        };
        items
            .iter()
            .filter_map(|item| {
                let url = item.stringForType(unsafe { NSPasteboardTypeFileURL })?;
                let url = NSURL::URLWithString(&url)?;
                let path = url.filePathURL()?.path()?;
                Some(path.to_string())
            })
            .collect()
    }
}
