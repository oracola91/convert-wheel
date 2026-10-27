//! Watches for a file drag with Shift held and shows the wheel under the cursor: the
//! formats, with Option (Alt) held too the tools, with Control the actions. While the
//! wheel is open the keys keep choosing, so one drag can move between the wheels.
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
use crate::wheel::{self, Hover, KeyFollower, WheelState, Which};

#[cfg(target_os = "macos")]
const POLL: Duration = Duration::from_millis(30);
/// Polls Shift must be held before the wheel opens, so Option (Alt) or Control pressed
/// a moment after it still picks the Tools or Actions wheel.
pub(crate) const KEYS_SETTLE: u8 = 2;
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
    /// "formats", "tools" or "actions": for the Tools wheel `formats` holds tool ids
    /// ("tool:half"), for the Actions wheel action ids ("act:path").
    wheel: Which,
    /// The dragged files' total size, shown in the middle; None when a folder is
    /// among them or a size can't be read.
    bytes: Option<u64>,
}

/// The total size of `paths`, or None with a folder among them.
fn total_bytes(paths: &[String]) -> Option<u64> {
    paths.iter().try_fold(0u64, |sum, p| {
        let meta = std::fs::metadata(p).ok()?;
        meta.is_file().then(|| sum.saturating_add(meta.len()))
    })
}

#[cfg(target_os = "macos")]
pub fn start(app: AppHandle) {
    thread::spawn(move || {
        let mut last_count = macos::drag_change_count();
        // The drag pasteboard changed while the button was down: a drag is under way.
        let mut dragging = false;
        let mut shown = false;
        // Polls in a row with Shift held during this drag.
        let mut held: u8 = 0;
        let mut keys = KeyFollower::default();
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
                    held = 0;
                    keys = KeyFollower::default();
                    return;
                }
                held = if macos::shift_down() { held.saturating_add(1) } else { 0 };
                if dragging && !shown && held >= KEYS_SETTLE {
                    let paths = macos::dragged_files();
                    // Not a file drag (text, a tab, ...): ignore it until the next drag.
                    if paths.is_empty() {
                        dragging = false;
                        return;
                    }
                    show_wheel(&app, paths, Which::from_keys(macos::option_down(), macos::control_down()));
                    shown = true;
                } else if shown {
                    follow_keys(&app, &mut keys, macos::shift_down(), macos::option_down(), macos::control_down());
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

/// What the wheel offers for `paths`: formats the user left on the wheel, or tools.
fn items_for(app: &AppHandle, paths: &[String], which: Which) -> Vec<&'static str> {
    match which {
        Which::Formats => {
            let settings = app.state::<crate::settings::Store>().get();
            formats::formats_for(paths, |kind, f| settings.shows(kind, f))
        }
        Which::Tools => formats::tools_for(paths),
        Which::Actions => formats::actions_for(paths),
    }
}

pub(crate) fn show_wheel(app: &AppHandle, paths: Vec<String>, which: Which) {
    let pending = paths.is_empty();
    let generation = open_wheel(app, paths, false, which);
    #[cfg(any(windows, target_os = "linux"))]
    if pending {
        close_later(app, generation, PENDING_TIMEOUT, true);
    }
    let _ = (generation, pending);
}

/// Opens the wheel from the keyboard shortcut and closes it again if no file comes.
#[cfg(target_os = "linux")]
pub(crate) fn show_pinned(app: &AppHandle) {
    let generation = open_wheel(app, Vec::new(), true, Which::Formats);
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

/// Puts on the wheel what `which` (or, with None, the wheel already chosen) offers for
/// `paths`. All under the `which` lock, so a switch and the files arriving (Windows,
/// Linux) can't leave one wheel's items on another.
fn fill(app: &AppHandle, paths: &[String], which: Option<Which>) -> (Which, Vec<&'static str>) {
    let state = app.state::<WheelState>();
    let mut current = wheel::lock(&state.which);
    if let Some(which) = which {
        *current = which;
    }
    let formats = items_for(app, paths, *current);
    *wheel::lock(&state.formats) = formats.clone();
    *wheel::lock(&state.paths) = paths.to_vec();
    (*current, formats)
}

/// One poll of the keys while the wheel is open: switches it when they ask for another.
pub(crate) fn follow_keys(app: &AppHandle, keys: &mut KeyFollower, shift: bool, option: bool, control: bool) {
    let current = *wheel::lock(&app.state::<WheelState>().which);
    if let Some(which) = keys.poll(current, KEYS_SETTLE, shift, option, control) {
        switch_wheel(app, which);
    }
}

/// Shows another wheel for the same drag, without closing the window.
fn switch_wheel(app: &AppHandle, which: Which) {
    let state = app.state::<WheelState>();
    if !state.is_open() {
        return;
    }
    let paths = wheel::lock(&state.paths).clone();
    let (which, formats) = fill(app, &paths, Some(which));
    let pending = paths.is_empty();
    let bytes = total_bytes(&paths);
    let _ = app.emit("wheel-formats", WheelShow { paths, formats, pending, glass: false, pinned: false, wheel: which, bytes });
}

/// Opens the wheel and returns its generation.
fn open_wheel(app: &AppHandle, paths: Vec<String>, pinned: bool, which: Which) -> u64 {
    let (_, formats) = fill(app, &paths, Some(which));
    let state = app.state::<WheelState>();
    let pending = paths.is_empty();
    let generation = state.open(pending);
    let first = paths.first().cloned();
    let glass = cfg!(target_os = "macos");
    let bytes = total_bytes(&paths);
    let _ = app.emit("wheel-show", WheelShow { paths, formats, pending, glass, pinned, wheel: which, bytes });
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
/// were being dragged: work out their formats (or tools) and redraw the wheel.
#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn files_entered(app: &AppHandle, paths: Vec<String>) {
    let (which, formats) = fill(app, &paths, None);
    app.state::<WheelState>().files_known();
    let bytes = total_bytes(&paths);
    let _ = app.emit("wheel-formats", WheelShow { paths, formats, pending: false, glass: false, pinned: false, wheel: which, bytes });
}

/// A drop on the wheel. Converts (or runs the tool) when it landed on a segment and
/// says whether it did.
pub(crate) fn drop_on_wheel(app: &AppHandle, hover: Hover, paths: Vec<String>) -> bool {
    let format = hover
        .segment
        .and_then(|i| wheel::lock(&app.state::<WheelState>().formats).get(i).copied());
    match format {
        // Crop first opens its window; the job is queued from there.
        Some(tool) if !paths.is_empty() && formats::opens_editor(tool) => {
            hide_wheel(app, true, None);
            crate::editor::open(app, paths);
            true
        }
        // Actions are quick and convert nothing: they skip the conversion queue.
        Some(action) if !paths.is_empty() && formats::is_action(action) => {
            hide_wheel(app, true, None);
            crate::actions::run(app, action, paths);
            true
        }
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

    pub fn option_down() -> bool {
        NSEvent::modifierFlags_class().contains(NSEventModifierFlags::Option)
    }

    pub fn control_down() -> bool {
        NSEvent::modifierFlags_class().contains(NSEventModifierFlags::Control)
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
