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
/// Polls between checks that a showing shelf still sits right, after screens changed.
#[cfg(target_os = "macos")]
const SHELF_REFRESH: u32 = 66;
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
/// How long the keyboard wheel waits for a choice before it closes by itself.
const KEYBOARD_TIMEOUT: Duration = Duration::from_secs(60);

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
    /// The menu's highlight colour ("blue"). Only on wheel-show: a switch keeps it.
    #[serde(skip_serializing_if = "Option::is_none")]
    highlight: Option<&'static str>,
    /// How the wheel looks, from the menu's Wheel Glass. Only on wheel-show: a switch
    /// keeps it.
    #[serde(skip_serializing_if = "Option::is_none")]
    look: Option<Look>,
}

/// Wheel Glass for the page: a plain solid disc, or Liquid Glass with this much white
/// on the segments. The frosted patches behind the words are native (wheel_window.rs).
#[derive(Clone, Copy, Serialize)]
struct Look {
    plain: bool,
    seg: f64,
    /// Windows and Linux draw the disc themselves: with Wheel Glass on, it is this
    /// opaque (see-through, no blur); None for macOS's glass or a solid disc.
    see: Option<f64>,
}

fn look(app: &AppHandle) -> Look {
    let settings = app.state::<crate::settings::Store>().get();
    let see = (!cfg!(target_os = "macos") && !settings.no_liquid_glass).then(|| settings.transparency.disc());
    Look { plain: settings.no_liquid_glass, seg: settings.transparency.paint(), see }
}

/// The total size of `paths`, or None with a folder among them.
fn total_bytes(paths: &[String]) -> Option<u64> {
    paths.iter().try_fold(0u64, |sum, p| {
        let meta = std::fs::metadata(p).ok()?;
        meta.is_file().then(|| sum.saturating_add(meta.len()))
    })
}

/// macOS: the arrow keys are read only once "Arrow Keys on the Wheel" is on (Keys
/// menu), as reading the keys of the app in front needs Input Monitoring. Windows and
/// Linux read them always.
#[cfg(target_os = "macos")]
static MAC_ARROWS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Switches the arrow keys on or off (macOS): at the start from the settings, and from
/// the menu.
#[cfg(target_os = "macos")]
pub(crate) fn set_mac_arrows(on: bool) {
    MAC_ARROWS.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Whether Input Monitoring is allowed; if not, macOS asks for it (a dialog the first
/// time only, after that the switch is in System Settings). Only called when the
/// arrow keys are switched on, never by the wheel itself.
#[cfg(target_os = "macos")]
pub(crate) fn ask_input_monitoring() -> bool {
    macos::input_monitoring_or_ask()
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
        let mut arrows = Arrows::default();
        // This drag carries files, so the shelf follows it (until the wheel opens), and
        // whether that was checked yet.
        let mut shelf_drag = false;
        let mut checked = false;
        let mut idle: u32 = 0;
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
                    checked = false;
                }
                if !button_down {
                    if shown {
                        released(&app);
                    }
                    if shelf_drag {
                        crate::shelf_window::drag_ended_soon(&app);
                    }
                    dragging = false;
                    shown = false;
                    held = 0;
                    keys = KeyFollower::default();
                    arrows = Arrows::default();
                    shelf_drag = false;
                    idle = (idle + 1) % SHELF_REFRESH;
                    if idle == 0 {
                        crate::shelf_window::refresh_soon(&app);
                    }
                    return;
                }
                if dragging && !checked {
                    checked = true;
                    shelf_drag = !macos::dragged_files().is_empty();
                }
                held = if macos::shift_down() { held.saturating_add(1) } else { 0 };
                if dragging && !shown && held >= KEYS_SETTLE {
                    // Keys switched off in the menu open no wheel; the drag goes on as usual.
                    if let Some(which) = Which::from_keys(macos::option_down(), macos::control_down()) {
                        let paths = macos::dragged_files();
                        // Not a file drag (text, a tab, ...): ignore it until the next drag.
                        if paths.is_empty() {
                            dragging = false;
                            return;
                        }
                        show_wheel(&app, paths, which);
                        shown = true;
                        // The wheel has this drag now: the shelf steps aside.
                        if shelf_drag {
                            shelf_drag = false;
                            crate::shelf_window::drag_ended_soon(&app);
                        }
                    }
                } else if shown {
                    follow_keys(&app, &mut keys, macos::shift_down(), macos::option_down(), macos::control_down());
                    if MAC_ARROWS.load(std::sync::atomic::Ordering::Relaxed) {
                        follow_arrows(&app, &mut arrows, macos::arrows());
                    }
                }
                if shelf_drag && dragging {
                    crate::shelf_window::dragging_soon(&app);
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

/// The keyboard wheel is open (keyboard mode): closing it hands the keyboard back.
static KEYBOARD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Keyboard mode (council 2026-11-10): the system hands over the selected files (the
/// Services entry on macOS, Send To on Windows, Open With on Linux) and the wheel
/// opens on them with the keyboard: Left and Right pick, Up and Down turn the set,
/// Enter applies, Esc closes, Option/Alt and Control switch wheels. It closes by
/// itself after KEYBOARD_TIMEOUT.
pub(crate) fn open_for_keys(app: &AppHandle, paths: Vec<String>) {
    let paths: Vec<String> = paths.into_iter().filter(|p| std::path::Path::new(p).exists()).collect();
    if paths.is_empty() {
        return;
    }
    crate::log::line(&format!("keyboard wheel on {} file(s)", paths.len()));
    KEYBOARD.store(true, std::sync::atomic::Ordering::SeqCst);
    let generation = open_wheel(app, paths, true, Which::Formats);
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        // After the wheel is shown (the same queue): the app comes to the front and
        // the wheel's page takes the keys.
        #[cfg(target_os = "macos")]
        if let Some(mtm) = objc2::MainThreadMarker::new() {
            #[allow(deprecated)]
            objc2_app_kit::NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
        }
        if let Some(win) = handle.get_webview_window("wheel") {
            let _ = win.set_focusable(true);
            let _ = win.set_focus();
        }
    });
    close_later(app, generation, KEYBOARD_TIMEOUT, false);
}

/// Enter on the keyboard wheel: the picked item, as a drop on it would be. On More,
/// the next set.
pub(crate) fn apply_pick(app: &AppHandle) {
    let state = app.state::<WheelState>();
    if !state.is_open() {
        return;
    }
    let Some(i) = *wheel::lock(&state.picked) else {
        return;
    };
    if wheel::lock(&state.formats).get(i).copied() == Some(wheel::MORE) {
        turn_page(app, true);
        return;
    }
    let paths = wheel::lock(&state.paths).clone();
    drop_on_wheel(app, Hover { segment: Some(i), hub: false }, paths);
}

/// The files a second start of the app was given (Send To, Open With), or None when
/// it was given anything else: every argument after the program must be a file or
/// folder that exists.
pub(crate) fn files_in_args(args: &[String]) -> Option<Vec<String>> {
    let given = args.get(1..)?;
    (!given.is_empty() && given.iter().all(|a| !a.starts_with('-') && std::path::Path::new(a).exists())).then(|| given.to_vec())
}

/// Opens the wheel from the keyboard shortcut and closes it again if no file comes.
/// `convert-wheel --show tools` (or actions) opens that wheel instead of the formats.
#[cfg(target_os = "linux")]
pub(crate) fn show_pinned(app: &AppHandle, which: Which) {
    let generation = open_wheel(app, Vec::new(), true, which);
    close_later(app, generation, PINNED_TIMEOUT, false);
}

/// Closes wheel `generation` after `delay`, unless it closed or a newer one opened
/// in the meantime. With `only_if_waiting`, only while it still has no files.
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
    let all = items_for(app, paths, *current);
    let formats = wheel::page_of(&all, 0, formats::MAX_SEGMENTS);
    *wheel::lock(&state.all) = all;
    *wheel::lock(&state.page) = 0;
    *wheel::lock(&state.formats) = formats.clone();
    *wheel::lock(&state.paths) = paths.to_vec();
    *wheel::lock(&state.picked) = None;
    (*current, formats)
}

/// One poll of the keys while the wheel is open: switches it when they ask for another.
pub(crate) fn follow_keys(app: &AppHandle, keys: &mut KeyFollower, shift: bool, option: bool, control: bool) {
    let current = *wheel::lock(&app.state::<WheelState>().which);
    if let Some(which) = keys.poll(&wheel::keys(), current, KEYS_SETTLE, shift, option, control) {
        switch_wheel(app, which);
    }
}

/// Arrow keys while the wheel is open: Left and Right move a highlight round the
/// wheel, Up and Down show the next or the previous set of a full wheel (More). A drop
/// on the hub then goes to the highlighted item; the mouse over a segment still wins.
/// Each system's loop reads the keys (left, right, up, down) and calls this.
#[derive(Default)]
pub(crate) struct Arrows {
    held: [bool; 4],
}

pub(crate) fn follow_arrows(app: &AppHandle, arrows: &mut Arrows, now: [bool; 4]) {
    for (key, (was, is)) in arrows.held.iter().zip(now).enumerate() {
        if is && !was {
            arrow(app, key);
        }
    }
    arrows.held = now;
}

/// One arrow key went down: 0 left, 1 right, 2 up, 3 down.
pub(crate) fn arrow(app: &AppHandle, key: usize) {
    let state = app.state::<WheelState>();
    if !state.is_open() {
        return;
    }
    match key {
        0 | 1 => {
            let n = wheel::lock(&state.formats).len();
            if n == 0 {
                return;
            }
            let next = {
                let mut picked = wheel::lock(&state.picked);
                let next = step(*picked, n, key == 1);
                *picked = Some(next);
                next
            };
            let _ = app.emit("wheel-pick", Some(next));
        }
        2 => turn_page(app, true),
        3 => turn_page(app, false),
        _ => {}
    }
}

/// The next segment round a wheel of `n`, clockwise or back; the first press starts
/// at the top (clockwise) or the last segment (back).
fn step(picked: Option<usize>, n: usize, clockwise: bool) -> usize {
    match picked {
        None if clockwise => 0,
        None => n - 1,
        Some(i) if clockwise => (i + 1) % n,
        Some(i) => (i + n - 1) % n,
    }
}

/// Whether a drop here would do something: on a segment, or on the hub once the arrow
/// keys picked one. The drop targets (macOS, Windows) take the file only then, so
/// anywhere else the system slides it back.
#[cfg(any(target_os = "macos", windows))]
pub(crate) fn accepts_drop(app: &AppHandle, hover: Hover) -> bool {
    with_pick(app, hover).segment.is_some()
}

/// macOS: the drag ended over the wheel without a drop, for example Esc: the wheel
/// closes now, not only once the mouse button goes up. After a drop it closed already,
/// so this does nothing then.
#[cfg(target_os = "macos")]
pub(crate) fn drag_ended_on_wheel(app: &AppHandle) {
    let generation = app.state::<WheelState>().generation();
    hide_wheel(app, false, Some(generation));
}

/// A drop on the hub goes to the segment the arrow keys picked.
fn with_pick(app: &AppHandle, hover: Hover) -> Hover {
    if hover.segment.is_none() && hover.hub {
        if let Some(i) = *wheel::lock(&app.state::<WheelState>().picked) {
            return Hover { segment: Some(i), hub: false };
        }
    }
    hover
}

/// The wheel is open on this one file.
pub(crate) fn showing(app: &AppHandle, path: &str) -> bool {
    let state = app.state::<WheelState>();
    state.is_open() && matches!(wheel::lock(&state.paths).as_slice(), [p] if p == path)
}

/// The shortcut's wheel has the keyboard (Linux): Alt and Control choose the wheel as
/// Option and Control do during a Shift+drag, by the keys set in the menu's Keys.
pub(crate) fn pinned_keys(app: &AppHandle, alt: bool, control: bool) {
    use crate::settings::Combo;
    let keys = app.state::<crate::settings::Store>().get().keys;
    let combo = if control {
        Combo::ShiftControl
    } else if alt {
        Combo::ShiftOption
    } else {
        Combo::Shift
    };
    if let Some(which) = [Which::Formats, Which::Tools, Which::Actions].into_iter().find(|w| keys.get(*w) == Some(combo)) {
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
    let _ = app.emit("wheel-formats", WheelShow { paths, formats, pending, glass: false, pinned: false, wheel: which, bytes, highlight: None, look: None });
    crate::estimates::ask(app);
}

/// The cursor rested on More: the same wheel shows its next set of items. Under the
/// `which` lock, like fill, so a switch or the files arriving can't cross it.
pub(crate) fn next_page(app: &AppHandle) {
    turn_page(app, true);
}

/// The next set of a full wheel, or with `forward` false the one before.
fn turn_page(app: &AppHandle, forward: bool) {
    let state = app.state::<WheelState>();
    if !state.is_open() {
        return;
    }
    let (which, formats, paths) = {
        let current = wheel::lock(&state.which);
        let all = wheel::lock(&state.all);
        if wheel::pages(all.len(), formats::MAX_SEGMENTS) == 1 {
            return;
        }
        let mut page = wheel::lock(&state.page);
        let pages = wheel::pages(all.len(), formats::MAX_SEGMENTS);
        *page = if forward { (*page + 1) % pages } else { (*page + pages - 1) % pages };
        *wheel::lock(&state.picked) = None;
        let formats = wheel::page_of(&all, *page, formats::MAX_SEGMENTS);
        *wheel::lock(&state.formats) = formats.clone();
        (*current, formats, wheel::lock(&state.paths).clone())
    };
    let pending = paths.is_empty();
    let bytes = total_bytes(&paths);
    let _ = app.emit("wheel-formats", WheelShow { paths, formats, pending, glass: false, pinned: false, wheel: which, bytes, highlight: None, look: None });
    crate::estimates::ask(app);
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
    let highlight = app.state::<crate::settings::Store>().get().highlight.key();
    let _ = app.emit("wheel-show", WheelShow { paths, formats, pending, glass, pinned, wheel: which, bytes, highlight: Some(highlight), look: Some(look(app)) });
    crate::estimates::ask(app);
    if let Some(f) = first.as_deref() {
        crate::hub_picture::send(app, f);
    }
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
    if let Some(first) = paths.first() {
        crate::hub_picture::send(app, first);
    }
    let bytes = total_bytes(&paths);
    let _ = app.emit("wheel-formats", WheelShow { paths, formats, pending: false, glass: false, pinned: false, wheel: which, bytes, highlight: None, look: None });
    crate::estimates::ask(app);
}

/// A drop on the wheel. Converts (or runs the tool) when it landed on a segment and
/// says whether it did.
pub(crate) fn drop_on_wheel(app: &AppHandle, hover: Hover, paths: Vec<String>) -> bool {
    let hover = with_pick(app, hover);
    let format = hover
        .segment
        .and_then(|i| wheel::lock(&app.state::<WheelState>().formats).get(i).copied());
    match format {
        // More only shows the next items: a drop there does nothing, like the middle.
        Some(wheel::MORE) => {
            hide_wheel(app, false, None);
            false
        }
        // Crop first opens its window; the job is queued from there.
        Some(tool) if !paths.is_empty() && formats::opens_editor(tool) => {
            hide_wheel(app, true, None);
            crate::editor::open(app, tool, paths);
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
    let keyboard = KEYBOARD.swap(false, std::sync::atomic::Ordering::SeqCst);
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        // The keyboard wheel took the keys: the wheel can't take them during a drag
        // again, and on macOS the app in front before it gets them back.
        if keyboard {
            if let Some(win) = handle.get_webview_window("wheel") {
                let _ = win.set_focusable(false);
            }
            #[cfg(target_os = "macos")]
            if let Some(mtm) = objc2::MainThreadMarker::new() {
                objc2_app_kit::NSApplication::sharedApplication(mtm).deactivate();
            }
        }
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

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceKeyState(state: i32, key: u16) -> bool;
        fn CGPreflightListenEventAccess() -> bool;
        fn CGRequestListenEventAccess() -> bool;
    }

    /// True when Input Monitoring is allowed; otherwise asks for it and says false.
    pub fn input_monitoring_or_ask() -> bool {
        unsafe { CGPreflightListenEventAccess() || CGRequestListenEventAccess() }
    }

    /// Left, right, up, down. Keys of another app in front are only seen with Input
    /// Monitoring allowed (System Settings > Privacy & Security); this never asks.
    pub fn arrows() -> [bool; 4] {
        const COMBINED_SESSION: i32 = 0;
        [123, 124, 126, 125].map(|key| unsafe { CGEventSourceKeyState(COMBINED_SESSION, key) })
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

#[cfg(test)]
mod arrow_tests {
    use super::{files_in_args, step};

    #[test]
    fn only_existing_files_open_the_keyboard_wheel() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.png");
        std::fs::write(&file, b"x").unwrap();
        let file = file.to_string_lossy().into_owned();
        let args = |rest: &[&str]| std::iter::once("app".to_string()).chain(rest.iter().map(|s| s.to_string())).collect::<Vec<_>>();
        assert_eq!(files_in_args(&args(&[&file])), Some(vec![file.clone()]));
        assert_eq!(files_in_args(&args(&[])), None);
        assert_eq!(files_in_args(&args(&["--update"])), None);
        assert_eq!(files_in_args(&args(&[&file, "/no/such/file"])), None);
    }

    #[test]
    fn arrows_go_round_the_wheel_both_ways() {
        assert_eq!(step(None, 5, true), 0);
        assert_eq!(step(None, 5, false), 4);
        assert_eq!(step(Some(4), 5, true), 0);
        assert_eq!(step(Some(0), 5, false), 4);
        assert_eq!(step(Some(2), 5, true), 3);
    }
}
