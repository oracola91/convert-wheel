//! Watches for a file drag with Shift held and shows the wheel under the cursor.
//!
//! macOS has no "drag started" notification for other apps, so this polls three
//! things the window server shares with everyone: the drag pasteboard's change
//! count, the mouse buttons and the modifier keys. No permission prompt is needed.

use std::sync::atomic::Ordering;
use std::{thread, time::Duration};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::formats;
use crate::wheel::WheelState;

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
                    // Close only the wheel this drag opened, not one opened since.
                    let app = app.clone();
                    let generation = app.state::<WheelState>().generation.load(Ordering::SeqCst);
                    thread::spawn(move || {
                        thread::sleep(HIDE_DELAY);
                        if app.state::<WheelState>().generation.load(Ordering::SeqCst) == generation {
                            hide_wheel(&app, false);
                        }
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
    let settings = app.state::<crate::settings::Store>().get();
    let formats = formats::formats_for(&paths, |kind, f| settings.shows(kind, f));
    let state = app.state::<WheelState>();
    if let Ok(mut shown) = state.formats.lock() {
        *shown = formats.clone();
    }
    state.generation.fetch_add(1, Ordering::SeqCst);
    state.open.store(true, Ordering::SeqCst);
    let first = paths.first().cloned();
    let _ = app.emit("wheel-show", WheelShow { paths, formats });
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || crate::wheel_window::open(&handle, first.as_deref()));
}

/// Closes the wheel. `accepted` tells the page a format was chosen, so it can keep
/// that segment lit while the wheel shrinks away. Closing twice does nothing.
#[cfg(target_os = "macos")]
pub fn hide_wheel(app: &AppHandle, accepted: bool) {
    let state = app.state::<WheelState>();
    if !state.open.swap(false, Ordering::SeqCst) {
        return;
    }
    let generation = state.generation.load(Ordering::SeqCst);
    let _ = app.emit("wheel-close", accepted);
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || crate::wheel_window::close(&handle, generation));
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
