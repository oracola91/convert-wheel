//! Watches for a file drag with Shift held and shows the wheel under the cursor.
//!
//! macOS has no "drag started" notification for other apps, so this polls three
//! things the window server shares with everyone: the drag pasteboard's change
//! count, the mouse buttons and the modifier keys. No permission prompt is needed.
//! The Windows and Linux triggers live in win32.rs and linux.rs. Opening, closing
//! and dropping are shared.

use std::{thread, time::Duration};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::formats;
use crate::wheel::{self, Hover, WheelState};

#[cfg(target_os = "macos")]
const POLL: Duration = Duration::from_millis(30);
/// How long the wheel stays after the button is released, so a drop on it can land.
const HIDE_DELAY: Duration = Duration::from_millis(400);
/// A wheel opened before its files were known (Windows, Linux) closes if the drag
/// never reaches it, for example after a Shift+drag selection in an empty area.
#[cfg(any(windows, target_os = "linux"))]
const PENDING_TIMEOUT: Duration = Duration::from_secs(3);
/// How long a wheel opened from the shortcut waits for a file.
#[cfg(target_os = "linux")]
const PINNED_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Serialize)]
struct WheelShow {
    paths: Vec<String>,
    formats: Vec<&'static str>,
    /// The files aren't known yet (Windows and Linux learn them when the drag
    /// reaches the wheel).
    pending: bool,
    /// The system draws frosted glass behind the ring (macOS); otherwise the page
    /// draws its own solid disc.
    glass: bool,
    /// Opened from the keyboard shortcut: stays open until a drop, Esc or a click in
    /// the middle, instead of closing when the mouse button goes up.
    pinned: bool,
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
            // AppKit hands back autoreleased objects; without a pool on this thread
            // they would pile up for as long as the app runs.
            objc2::rc::autoreleasepool(|_| {
                let button_down = macos::left_button_down();
                let count = macos::drag_change_count();
                if count != last_count {
                    last_count = count;
                    dragging = button_down;
                }
                if !button_down {
                    if shown {
                        released(&app);
                    }
                    dragging = false;
                    shown = false;
                    return;
                }
                if dragging && !shown && macos::shift_down() {
                    let paths = macos::dragged_files();
                    // Not a file drag (text, a tab, ...): ignore it until the next drag.
                    if paths.is_empty() {
                        dragging = false;
                        return;
                    }
                    show_wheel(&app, paths);
                    shown = true;
                }
            });
        }
    });
}

#[cfg(windows)]
pub fn start(app: AppHandle) {
    crate::win32::start_trigger(app);
}

#[cfg(target_os = "linux")]
pub fn start(app: AppHandle) {
    crate::linux::start_trigger(app);
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
pub fn start(_app: AppHandle) {}

fn formats_for(app: &AppHandle, paths: &[String]) -> Vec<&'static str> {
    let settings = app.state::<crate::settings::Store>().get();
    formats::formats_for(paths, |kind, f| settings.shows(kind, f))
}

pub(crate) fn show_wheel(app: &AppHandle, paths: Vec<String>) {
    let pending = paths.is_empty();
    let generation = open_wheel(app, paths, false);
    #[cfg(any(windows, target_os = "linux"))]
    if pending {
        close_later(app, generation, PENDING_TIMEOUT, true);
    }
    let _ = (generation, pending);
}

/// Opens the wheel from the keyboard shortcut and closes it again if no file comes.
#[cfg(target_os = "linux")]
pub(crate) fn show_pinned(app: &AppHandle) {
    let generation = open_wheel(app, Vec::new(), true);
    close_later(app, generation, PINNED_TIMEOUT, false);
}

/// Closes wheel `generation` after `delay`, unless it closed or a newer one opened
/// in the meantime. With `only_if_waiting`, only while it still has no files.
#[cfg(any(windows, target_os = "linux"))]
fn close_later(app: &AppHandle, generation: u64, delay: Duration, only_if_waiting: bool) {
    let app = app.clone();
    thread::spawn(move || {
        thread::sleep(delay);
        if !only_if_waiting || app.state::<WheelState>().still_waiting(generation) {
            hide_wheel(&app, false, Some(generation));
        }
    });
}

/// Opens the wheel and returns its generation.
fn open_wheel(app: &AppHandle, paths: Vec<String>, pinned: bool) -> u64 {
    let formats = formats_for(app, &paths);
    let state = app.state::<WheelState>();
    *wheel::lock(&state.formats) = formats.clone();
    let pending = paths.is_empty();
    let generation = state.open(pending);
    let first = paths.first().cloned();
    let glass = cfg!(target_os = "macos");
    let _ = app.emit("wheel-show", WheelShow { paths, formats, pending, glass, pinned });
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        #[cfg(target_os = "macos")]
        crate::wheel_window::open(&handle, first.as_deref());
        #[cfg(windows)]
        crate::win32::open(&handle);
        #[cfg(target_os = "linux")]
        crate::linux::open(&handle, pinned);
        let _ = (&handle, &first, pinned);
    });
    generation
}

/// The files reached the wheel on a system that couldn't tell earlier which files
/// were being dragged: work out their formats and redraw the wheel.
#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn files_entered(app: &AppHandle, paths: Vec<String>) {
    let formats = formats_for(app, &paths);
    let state = app.state::<WheelState>();
    *wheel::lock(&state.formats) = formats.clone();
    state.files_known();
    let _ = app.emit("wheel-formats", WheelShow { paths, formats, pending: false, glass: false, pinned: false });
}

/// A drop on the wheel. Converts when it landed on a format and says whether it did.
pub(crate) fn drop_on_wheel(app: &AppHandle, hover: Hover, paths: Vec<String>) -> bool {
    let format = hover
        .segment
        .and_then(|i| wheel::lock(&app.state::<WheelState>().formats).get(i).copied());
    match format {
        Some(format) if !paths.is_empty() => {
            hide_wheel(app, true, None);
            crate::enqueue(app, paths, format.to_string());
            true
        }
        Some(_) => {
            // Dropped on a format, but nothing came with it that is a file on this
            // computer (an email attachment or a link, for example).
            hide_wheel(app, false, None);
            crate::notify(app, "Nothing to convert", "Drag files from a folder or the desktop.");
            false
        }
        None => {
            hide_wheel(app, false, None);
            false
        }
    }
}

/// The mouse button went up during a drag that opened the wheel. Closes that wheel
/// shortly after, unless it closed already or a newer one has opened since.
pub(crate) fn released(app: &AppHandle) {
    let app = app.clone();
    let generation = app.state::<WheelState>().generation();
    thread::spawn(move || {
        thread::sleep(HIDE_DELAY);
        hide_wheel(&app, false, Some(generation));
    });
}

/// Closes the wheel. `accepted` tells the page a format was chosen, so it can keep
/// that segment lit while the wheel shrinks away. With `expected`, only that wheel.
/// Closing twice does nothing.
pub fn hide_wheel(app: &AppHandle, accepted: bool, expected: Option<u64>) {
    let Some(generation) = app.state::<WheelState>().close(expected) else {
        return;
    };
    let _ = app.emit("wheel-close", accepted);
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        #[cfg(target_os = "macos")]
        crate::wheel_window::close(&handle, generation);
        #[cfg(windows)]
        crate::win32::close(&handle, generation);
        #[cfg(target_os = "linux")]
        crate::linux::close(&handle, generation);
        let _ = (&handle, generation);
    });
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
