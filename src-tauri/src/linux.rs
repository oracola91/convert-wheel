//! Linux: Shift+drag detection on X11, a keyboard-shortcut wheel on Wayland, the
//! wheel window's placement, and its drop target.
//!
//! X11 lets any app read the pointer and modifier keys, so it works like Windows: a
//! drag that starts in a file manager with Shift held opens the wheel. Wayland keeps
//! the pointer private, so there the wheel opens from a keyboard shortcut instead
//! (`convert-wheel --show`, bound in the desktop's keyboard settings) and stays open
//! until a file is dropped on it. The shortcut works on X11 too.

use std::cell::RefCell;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::rc::Rc;
use std::{fs, thread, time::Duration};

use gtk::prelude::*;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, KeyButMask, Window};
use x11rb::rust_connection::RustConnection;

use crate::trigger;
use crate::wheel::{self, Hover, WheelState, Which};

const POLL: Duration = Duration::from_millis(30);
/// Matches the close animation in src/wheel.css.
const CLOSE: Duration = Duration::from_millis(140);
/// GTK's default distance before a press counts as a drag.
const DRAG_THRESHOLD: i32 = 8;

/// Window class names of file managers and desktops, matched whole and in lower case.
const FILE_MANAGERS: &[&str] = &[
    "nautilus", "org.gnome.nautilus", "nemo", "caja", "thunar", "dolphin", "pcmanfm", "pcmanfm-qt", "spacefm",
    "konqueror", "xfdesktop", "ding", "plasmashell",
];

pub fn is_wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t.eq_ignore_ascii_case("wayland"))
}

pub fn start_trigger(app: AppHandle) {
    let shortcut = start_listener(app.clone());
    if !is_wayland() {
        start_x11(app);
    } else if !shortcut {
        // On Wayland the shortcut is the only way in; without it nothing opens the wheel.
        crate::notify(
            &app,
            "Convert Wheel can't open its wheel here",
            "This desktop session has no XDG_RUNTIME_DIR, which the keyboard shortcut needs. Details are in the log.",
        );
    }
}

/// Polls the pointer through a connection of its own. Every X error comes back as a
/// value here, so a window that closes between two calls just ends that look, and a
/// lost connection ends the trigger instead of the app.
fn start_x11(app: AppHandle) {
    thread::spawn(move || {
        let (conn, screen) = match x11rb::connect(None) {
            Ok(c) => c,
            Err(e) => {
                crate::log::line(&format!("no X display ({e}); Shift+drag is off, the --show shortcut still works"));
                return;
            }
        };
        let root = conn.setup().roots[screen].root;
        let (mut was_down, mut start, mut from_files, mut shown) = (false, (0, 0), false, false);
        // Polls in a row with Shift held during this drag.
        let mut held: u8 = 0;
        loop {
            thread::sleep(POLL);
            let pointer = match conn.query_pointer(root).map(|c| c.reply()) {
                Ok(Ok(p)) => p,
                Ok(Err(_)) => continue,
                Err(e) => {
                    crate::log::line(&format!("X connection lost ({e}); Shift+drag is off until restart"));
                    return;
                }
            };
            // X reports buttons after any left/right swap, so button 1 is the dragging one.
            if !pointer.mask.contains(KeyButMask::BUTTON1) {
                if shown {
                    trigger::released(&app);
                }
                (was_down, from_files, shown, held) = (false, false, false, 0);
                continue;
            }
            let (px, py) = (i32::from(pointer.root_x), i32::from(pointer.root_y));
            if !was_down {
                was_down = true;
                start = (px, py);
                from_files = starts_in_file_manager(&conn, root);
            }
            let moved = (px - start.0).abs() > DRAG_THRESHOLD || (py - start.1).abs() > DRAG_THRESHOLD;
            held = if pointer.mask.contains(KeyButMask::SHIFT) { held.saturating_add(1) } else { 0 };
            if from_files && !shown && held >= trigger::KEYS_SETTLE && moved {
                // MOD1 is Alt.
                let which = Which::from_keys(pointer.mask.contains(KeyButMask::MOD1));
                trigger::show_wheel(&app, Vec::new(), which);
                shown = true;
            }
        }
    });
}

/// Walks down from the root to the first window under the pointer that names its
/// app (the window manager's frame around it has no name) and checks the name.
fn starts_in_file_manager(conn: &RustConnection, root: Window) -> bool {
    let mut window = root;
    for _ in 0..8 {
        let Ok(Ok(pointer)) = conn.query_pointer(window).map(|c| c.reply()) else {
            return false;
        };
        if pointer.child == x11rb::NONE {
            return false;
        }
        if let Some(names) = class_names(conn, pointer.child) {
            return names.iter().any(|n| FILE_MANAGERS.contains(&n.as_str()));
        }
        window = pointer.child;
    }
    false
}

/// The two names in a window's WM_CLASS (instance and class), in lower case.
fn class_names(conn: &RustConnection, window: Window) -> Option<Vec<String>> {
    let reply = conn
        .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)
        .ok()?
        .reply()
        .ok()?;
    if reply.value.is_empty() {
        return None;
    }
    Some(
        reply
            .value
            .split(|&b| b == 0)
            .filter(|part| !part.is_empty())
            .map(|part| String::from_utf8_lossy(part).to_lowercase())
            .collect(),
    )
}

/// Where the running app listens for `convert-wheel --show`: the user's private
/// runtime folder. Without one there is no listener: a shared folder such as /tmp
/// would let other users squat the name.
fn socket_path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(|d| PathBuf::from(d).join("convert-wheel.sock"))
}

/// Starts the listener for the shortcut; false if there is none.
fn start_listener(app: AppHandle) -> bool {
    let Some(path) = socket_path() else {
        crate::log::line("XDG_RUNTIME_DIR is not set; the --show shortcut is off");
        return false;
    };
    // SAFETY: getuid has no preconditions and can't fail.
    let me = unsafe { libc::getuid() };
    // A socket left by a crash blocks the name. Remove it only if nobody answers on it
    // and it really is our own socket.
    if UnixStream::connect(&path).is_err() {
        let ours = fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_socket() && m.uid() == me);
        if ours {
            let _ = fs::remove_file(&path);
        }
    }
    // Private from the moment it exists, not only after a chmod.
    // SAFETY: umask only changes this process's file-creation mask; it is put back below.
    let old_mask = unsafe { libc::umask(0o177) };
    let bound = UnixListener::bind(&path);
    unsafe { libc::umask(old_mask) };
    let listener = match bound {
        Ok(l) => l,
        Err(e) => {
            crate::log::line(&format!("can't listen for --show ({e})"));
            return false;
        }
    };
    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            // A client that connects and says nothing must not block the next one.
            let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
            let mut line = String::new();
            let _ = BufReader::new(stream.take(64)).read_line(&mut line);
            if line.trim() == "show" {
                trigger::show_pinned(&app);
            }
        }
    });
    true
}

/// `convert-wheel --show`: asks the running app to open the wheel.
pub fn send_show() -> Result<(), String> {
    let path = socket_path().ok_or("XDG_RUNTIME_DIR is not set")?;
    UnixStream::connect(path)
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
            if handle.state::<WheelState>().is_current(generation) {
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
        // Ask for the file list once per drag. The borrow ends first, in case GTK
        // answers straight away.
        let ask = !std::mem::replace(&mut s.borrow_mut().requested, true);
        if ask {
            w.drag_get_data(ctx, &list, time);
        }
        let hover = hover_at(&a, x, y);
        let mut st = s.borrow_mut();
        set_hover(&a, &mut st, hover);
        // Copy over a format, nothing anywhere else. Never "move", even with Shift held,
        // and only if the source allows a copy.
        let can_copy = ctx.actions().contains(gdk::DragAction::COPY);
        let action = if hover.segment.is_some() && !st.paths.is_empty() && can_copy {
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
        let (waiting, hover) = {
            let mut st = s.borrow_mut();
            st.paths = paths.clone();
            (st.waiting_drop.take(), st.hover)
        };
        if !paths.is_empty() {
            trigger::files_entered(&a, paths.clone());
        }
        if let Some(drop_time) = waiting {
            finish(&a, ctx, x, y, paths, drop_time);
            return;
        }
        // The first motion answered "no" because the files weren't known yet; answer
        // again now, so a drop made without moving the mouse isn't refused.
        let can_copy = ctx.actions().contains(gdk::DragAction::COPY);
        let over_format = hover.is_some_and(|h| h.segment.is_some());
        let action = if over_format && !paths.is_empty() && can_copy {
            gdk::DragAction::COPY
        } else {
            gdk::DragAction::empty()
        };
        ctx.drag_status(action, time);
    });

    let (a, s, list) = (app.clone(), state.clone(), uri_list);
    webview.connect_drag_drop(move |w, ctx, x, y, time| {
        let paths = s.borrow().paths.clone();
        if paths.is_empty() {
            // GTK sends "leave" just before "drop", which cleared the list: ask again.
            s.borrow_mut().waiting_drop = Some(time);
            w.drag_get_data(ctx, &list, time);
            // If the list never comes, release the drop so the file manager isn't left
            // waiting.
            let (a, s, ctx) = (a.clone(), s.clone(), ctx.clone());
            glib::timeout_add_local_once(Duration::from_secs(2), move || {
                let stuck = s.borrow_mut().waiting_drop.take();
                if let Some(drop_time) = stuck {
                    crate::log::line("drop released: the file list never arrived");
                    trigger::hide_wheel(&a, false, None);
                    ctx.drag_finish(false, false, drop_time);
                }
            });
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
    let n = wheel::lock(&app.state::<WheelState>().formats).len();
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
