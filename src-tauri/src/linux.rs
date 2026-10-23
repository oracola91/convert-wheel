//! Linux: Shift+drag detection on X11, a keyboard-shortcut wheel on Wayland, the
//! wheel window's placement, and its drop target.
//!
//! X11 lets any app read the pointer and modifier keys, so it works like Windows: a
//! drag that starts in a file manager with Shift held opens the wheel. Wayland keeps
//! the pointer private, so there the wheel opens from a keyboard shortcut instead
//! (`convert-wheel --show`, bound in the desktop's keyboard settings) and stays open
//! until a file is dropped on it. The shortcut works on X11 too.

use std::cell::RefCell;
use std::ffi::CStr;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::{fs, ptr, thread, time::Duration};

use gtk::prelude::*;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition};
use x11_dl::xlib::{self, Xlib};

use crate::trigger;
use crate::wheel::{self, Hover, WheelState};

const POLL: Duration = Duration::from_millis(30);
/// Matches the close animation in src/wheel.css.
const CLOSE: Duration = Duration::from_millis(140);
/// GTK's default distance before a press counts as a drag.
const DRAG_THRESHOLD: i32 = 8;

/// Window classes of file managers and desktops (lower case, matched as substrings).
const FILE_MANAGERS: &[&str] = &[
    "nautilus", "nemo", "caja", "thunar", "dolphin", "pcmanfm", "spacefm", "konqueror", "xfdesktop", "ding",
    "desktopicons", "plasmashell",
];

pub fn is_wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t.eq_ignore_ascii_case("wayland"))
}

pub fn start_trigger(app: AppHandle) {
    start_listener(app.clone());
    if !is_wayland() {
        start_x11(app);
    }
}

fn start_x11(app: AppHandle) {
    thread::spawn(move || {
        let Ok(x) = Xlib::open() else {
            eprintln!("convert-wheel: libX11 not found, Shift+drag is off");
            return;
        };
        // A connection of our own, used only by this thread.
        let display = unsafe { (x.XOpenDisplay)(ptr::null()) };
        if display.is_null() {
            eprintln!("convert-wheel: no X display, Shift+drag is off");
            return;
        }
        let root = unsafe { (x.XDefaultRootWindow)(display) };
        let (mut was_down, mut start, mut from_files, mut shown) = (false, (0, 0), false, false);
        loop {
            thread::sleep(POLL);
            let Some((px, py, mask, _)) = pointer(&x, display, root) else {
                continue;
            };
            // X reports buttons after any left/right swap, so button 1 is the dragging one.
            if mask & xlib::Button1Mask == 0 {
                if shown {
                    trigger::released(&app);
                }
                (was_down, from_files, shown) = (false, false, false);
                continue;
            }
            if !was_down {
                was_down = true;
                start = (px, py);
                from_files = starts_in_file_manager(&x, display, root);
            }
            let moved = (px - start.0).abs() > DRAG_THRESHOLD || (py - start.1).abs() > DRAG_THRESHOLD;
            if from_files && !shown && mask & xlib::ShiftMask != 0 && moved {
                trigger::show_wheel(&app, Vec::new());
                shown = true;
            }
        }
    });
}

/// Pointer position on the screen, the button and modifier mask, and the child of
/// `window` that the pointer is over (0 for none).
fn pointer(x: &Xlib, display: *mut xlib::Display, window: xlib::Window) -> Option<(i32, i32, u32, xlib::Window)> {
    let (mut root, mut child, mut rx, mut ry, mut wx, mut wy, mut mask) = (0, 0, 0, 0, 0, 0, 0);
    let ok = unsafe {
        (x.XQueryPointer)(display, window, &mut root, &mut child, &mut rx, &mut ry, &mut wx, &mut wy, &mut mask)
    };
    (ok != 0).then_some((rx, ry, mask, child))
}

/// Walks down from the root to the first window under the pointer that names its
/// app (the window manager's frame around it has no name) and checks the name.
fn starts_in_file_manager(x: &Xlib, display: *mut xlib::Display, root: xlib::Window) -> bool {
    let mut window = root;
    for _ in 0..8 {
        let Some((_, _, _, child)) = pointer(x, display, window) else {
            return false;
        };
        if child == 0 {
            return false;
        }
        if let Some(class) = class_hint(x, display, child) {
            return FILE_MANAGERS.iter().any(|f| class.contains(f));
        }
        window = child;
    }
    false
}

fn class_hint(x: &Xlib, display: *mut xlib::Display, window: xlib::Window) -> Option<String> {
    let mut hint = xlib::XClassHint { res_name: ptr::null_mut(), res_class: ptr::null_mut() };
    if unsafe { (x.XGetClassHint)(display, window, &mut hint) } == 0 {
        return None;
    }
    let mut class = String::new();
    for part in [hint.res_name, hint.res_class] {
        if !part.is_null() {
            class.push_str(&unsafe { CStr::from_ptr(part) }.to_string_lossy().to_lowercase());
            class.push(' ');
            unsafe { (x.XFree)(part.cast()) };
        }
    }
    Some(class)
}

/// Where the running app listens for `convert-wheel --show`. XDG_RUNTIME_DIR is the
/// user's own private folder.
fn socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("convert-wheel.sock")
}

fn start_listener(app: AppHandle) {
    let path = socket_path();
    // A socket file left by a crash blocks the name; nobody answers on it, so it can go.
    if UnixStream::connect(&path).is_err() {
        let _ = fs::remove_file(&path);
    }
    let Ok(listener) = UnixListener::bind(&path) else {
        eprintln!("convert-wheel: can't listen on {}", path.display());
        return;
    };
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            let _ = BufReader::new(stream.take(64)).read_line(&mut line);
            if line.trim() == "show" {
                trigger::show_pinned(&app);
            }
        }
    });
}

/// `convert-wheel --show`: asks the running app to open the wheel.
pub fn send_show() -> Result<(), String> {
    UnixStream::connect(socket_path())
        .and_then(|mut s| s.write_all(b"show\n"))
        .map_err(|_| "Convert Wheel isn't running".into())
}

/// Shows the wheel. From Shift+drag on X11 it sits at the cursor, inside the screen's
/// work area, and doesn't take focus. From the shortcut it opens where the desktop
/// puts it (Wayland apps can't place their windows) and takes focus, so Esc closes it.
pub fn open(app: &AppHandle, pinned: bool) {
    let Some(win) = app.get_webview_window("wheel") else {
        return;
    };
    if pinned {
        let _ = win.set_focusable(true);
        let _ = win.center();
        let _ = win.show();
        let _ = win.set_focus();
        return;
    }
    let _ = win.set_focusable(false);
    if let Ok(cursor) = app.cursor_position() {
        if let Ok(Some(monitor)) = app.monitor_from_point(cursor.x, cursor.y) {
            let size = (wheel::SIZE * monitor.scale_factor()).round() as i32;
            let area = monitor.work_area();
            let (left, top) = (area.position.x, area.position.y);
            let (right, bottom) = (left + area.size.width as i32, top + area.size.height as i32);
            let x = (cursor.x as i32 - size / 2).min(right - size).max(left);
            let y = (cursor.y as i32 - size / 2).min(bottom - size).max(top);
            let _ = win.set_position(PhysicalPosition::new(x, y));
        }
    }
    let _ = win.show();
}

/// Lets the page's close animation play, then hides the wheel unless a newer one
/// opened in the meantime.
pub fn close(app: &AppHandle, generation: u64) {
    let app = app.clone();
    thread::spawn(move || {
        thread::sleep(CLOSE);
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            if handle.state::<WheelState>().generation.load(Ordering::SeqCst) == generation {
                if let Some(win) = handle.get_webview_window("wheel") {
                    let _ = win.hide();
                }
            }
        });
    });
}

#[derive(Default)]
struct DropState {
    paths: Vec<String>,
    /// The file list has been asked for during this drag.
    requested: bool,
    /// A drop arrived before the file list did: its time stamp, to finish it later.
    waiting_drop: Option<u32>,
    hover: Option<Hover>,
}

/// Takes over drag and drop on the wheel's web view from WebKit.
pub fn install(app: &AppHandle) {
    let Some(win) = app.get_webview_window("wheel") else {
        return;
    };
    let app = app.clone();
    let _ = win.with_webview(move |w| connect_drop(app, w.inner()));
}

fn connect_drop(app: AppHandle, webview: webkit2gtk::WebView) {
    let state = Rc::new(RefCell::new(DropState::default()));
    let uri_list = gdk::Atom::intern("text/uri-list");

    let (a, s, list) = (app.clone(), state.clone(), uri_list.clone());
    webview.connect_drag_motion(move |w, ctx, x, y, time| {
        let mut st = s.borrow_mut();
        if !st.requested {
            st.requested = true;
            w.drag_get_data(ctx, &list, time);
        }
        let hover = hover_at(&a, x, y);
        set_hover(&a, &mut st, hover);
        // Copy over a format, nothing anywhere else. Never "move", even with Shift held.
        let action = if hover.segment.is_some() && !st.paths.is_empty() {
            gdk::DragAction::COPY
        } else {
            gdk::DragAction::empty()
        };
        ctx.drag_status(action, time);
        true
    });

    let (a, s) = (app.clone(), state.clone());
    webview.connect_drag_data_received(move |w, ctx, x, y, data, _info, time| {
        w.stop_signal_emission_by_name("drag-data-received");
        let paths: Vec<String> = data
            .uris()
            .iter()
            .filter_map(|uri| glib::filename_from_uri(uri).ok())
            .map(|(path, _)| path.to_string_lossy().into_owned())
            .collect();
        let waiting = {
            let mut st = s.borrow_mut();
            st.paths = paths.clone();
            st.waiting_drop.take()
        };
        if !paths.is_empty() {
            trigger::files_entered(&a, paths.clone());
        }
        if let Some(drop_time) = waiting {
            finish(&a, ctx, x, y, paths, drop_time);
        }
    });

    let (a, s, list) = (app.clone(), state.clone(), uri_list);
    webview.connect_drag_drop(move |w, ctx, x, y, time| {
        let paths = s.borrow().paths.clone();
        if paths.is_empty() {
            // GTK sends "leave" just before "drop", which cleared the list: ask again.
            s.borrow_mut().waiting_drop = Some(time);
            w.drag_get_data(ctx, &list, time);
        } else {
            finish(&a, ctx, x, y, paths, time);
        }
        true
    });

    let (a, s) = (app, state);
    webview.connect_drag_leave(move |w, _ctx, _time| {
        w.stop_signal_emission_by_name("drag-leave");
        let mut st = s.borrow_mut();
        st.requested = false;
        st.paths.clear();
        set_hover(&a, &mut st, Hover::NONE);
    });
}

fn hover_at(app: &AppHandle, x: i32, y: i32) -> Hover {
    let n = app.state::<WheelState>().formats.lock().map(|f| f.len()).unwrap_or(0);
    wheel::hover_at(x as f64 - wheel::SIZE / 2.0, wheel::SIZE / 2.0 - y as f64, n)
}

fn set_hover(app: &AppHandle, st: &mut DropState, hover: Hover) {
    if st.hover != Some(hover) {
        st.hover = Some(hover);
        let _ = app.emit("wheel-hover", hover);
    }
}

fn finish(app: &AppHandle, ctx: &gdk::DragContext, x: i32, y: i32, paths: Vec<String>, time: u32) {
    let converted = trigger::drop_on_wheel(app, hover_at(app, x, y), paths);
    // `false`: the file manager must never delete the original.
    ctx.drag_finish(converted, false, time);
}
