//! Windows: Shift+drag detection, the wheel window's placement, and its drop target.
//!
//! Windows has no shared drag pasteboard to watch, so the trigger watches the mouse
//! button, Shift and where the drag started: a drag that begins in File Explorer, on
//! the desktop or in an Open/Save dialog with Shift held opens the wheel. Which files
//! are being dragged becomes known only when the drag reaches the wheel.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::{ptr, thread, time::Duration};

use tauri::{AppHandle, Emitter, Manager};
use windows::core::{implement, Ref, BOOL};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, POINTL};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, ScreenToClient, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
use windows::Win32::System::Ole::{
    IDropTarget, IDropTarget_Impl, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop, CF_HDROP, DROPEFFECT,
    DROPEFFECT_COPY, DROPEFFECT_NONE,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VIRTUAL_KEY, VK_LBUTTON, VK_RBUTTON, VK_SHIFT};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, GetAncestor, GetClassNameW, GetCursorPos, GetSystemMetrics, GetWindowLongPtrW, IsWindow,
    SetWindowLongPtrW,
    SetWindowPos, ShowWindow, WindowFromPoint, GA_PARENT, GWL_EXSTYLE, GWL_STYLE, HWND_TOPMOST, SM_CXDRAG,
    SM_CYDRAG, SM_SWAPBUTTON, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    SWP_SHOWWINDOW, SW_HIDE, WS_CAPTION, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_WINDOWEDGE, WS_MAXIMIZEBOX,
    WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
};

use crate::trigger;
use crate::wheel::{self, Hover, WheelState};

const POLL: Duration = Duration::from_millis(30);
/// Matches the close animation in src/wheel.css.
const CLOSE: Duration = Duration::from_millis(140);

/// Window classes that hold a file list: File Explorer, the desktop, and the
/// file view inside Open/Save dialogs.
const FILE_VIEWS: &[&str] = &["SHELLDLL_DefView", "CabinetWClass", "ExploreWClass", "Progman", "WorkerW"];

pub fn start_trigger(app: AppHandle) {
    thread::spawn(move || {
        let mut was_down = false;
        let mut start = POINT::default();
        let mut from_files = false;
        let mut shown = false;
        loop {
            thread::sleep(POLL);
            let mut pt = POINT::default();
            if unsafe { GetCursorPos(&mut pt) }.is_err() {
                continue;
            }
            if !key_down(primary_button()) {
                if shown {
                    trigger::released(&app);
                }
                (was_down, from_files, shown) = (false, false, false);
                continue;
            }
            if !was_down {
                was_down = true;
                start = pt;
                from_files = starts_in_file_view(pt);
            }
            if from_files && !shown && key_down(VK_SHIFT) && moved_past_threshold(start, pt) {
                trigger::show_wheel(&app, Vec::new());
                shown = true;
            }
        }
    });
}

fn key_down(key: VIRTUAL_KEY) -> bool {
    (unsafe { GetAsyncKeyState(key.0 as i32) } as u16) & 0x8000 != 0
}

/// The button that drags, which is the right one for people who swapped them.
fn primary_button() -> VIRTUAL_KEY {
    if unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0 {
        VK_RBUTTON
    } else {
        VK_LBUTTON
    }
}

/// Windows' own distance before a press counts as a drag.
fn moved_past_threshold(a: POINT, b: POINT) -> bool {
    let (dx, dy) = unsafe { (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG)) };
    (b.x - a.x).abs() > dx || (b.y - a.y).abs() > dy
}

fn starts_in_file_view(pt: POINT) -> bool {
    let mut hwnd = unsafe { WindowFromPoint(pt) };
    for _ in 0..16 {
        if hwnd.is_invalid() {
            return false;
        }
        if FILE_VIEWS.contains(&class_name(hwnd).as_str()) {
            return true;
        }
        hwnd = unsafe { GetAncestor(hwnd, GA_PARENT) };
    }
    false
}

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn wheel_hwnd(app: &AppHandle) -> Option<HWND> {
    app.get_webview_window("wheel")?.hwnd().ok()
}

/// Makes the wheel a window that never takes focus from File Explorer and never
/// shows in the taskbar or Alt+Tab.
pub fn install(app: &AppHandle) {
    if let Some(hwnd) = wheel_hwnd(app) {
        make_bare_popup(hwnd);
    }
}

/// A plain popup with no frame at all. Tauri keeps the title-bar style on borderless
/// windows and hides it itself; together with the tool-window style (no taskbar, no
/// Alt+Tab) Windows then drew a small grey title bar above the wheel. Without the
/// title-bar styles there is nothing to draw.
fn make_bare_popup(hwnd: HWND) {
    let frame = (WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX).0 as isize;
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        SetWindowLongPtrW(hwnd, GWL_STYLE, (style & !frame) | WS_POPUP.0 as isize);
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let ex = (ex | (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0) as isize) & !(WS_EX_WINDOWEDGE.0 as isize);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex);
        // Windows only re-reads the frame when told to.
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// Shows the wheel centred on the cursor, kept inside the screen's work area so the
/// taskbar never covers it. Runs on the main thread.
pub fn open(app: &AppHandle) {
    let Some(hwnd) = wheel_hwnd(app) else {
        return;
    };
    // Again on every open, in case Tauri reapplied its own styles in the meantime.
    make_bare_popup(hwnd);
    register_drop_targets(app, hwnd);
    let mut pt = POINT::default();
    let _ = unsafe { GetCursorPos(&mut pt) };
    let monitor = unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    let _ = unsafe { GetMonitorInfoW(monitor, &mut info) };
    let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
    let _ = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
    let size = (wheel::SIZE * dpi_x as f64 / 96.0).round() as i32;
    let work = info.rcWork;
    let x = (pt.x - size / 2).min(work.right - size).max(work.left);
    let y = (pt.y - size / 2).min(work.bottom - size).max(work.top);
    let _ = unsafe { SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, size, size, SWP_NOACTIVATE | SWP_SHOWWINDOW) };
}

/// Lets the page's close animation play, then hides the wheel unless a newer one
/// opened in the meantime.
pub fn close(app: &AppHandle, generation: u64) {
    let app = app.clone();
    thread::spawn(move || {
        thread::sleep(CLOSE);
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            if !handle.state::<WheelState>().is_current(generation) {
                return;
            }
            if let Some(hwnd) = wheel_hwnd(&handle) {
                let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
            }
        });
    });
}

thread_local! {
    // Main thread only: windows that already have our drop target, and the targets
    // themselves, kept alive for as long as the app runs.
    static REGISTERED: RefCell<HashSet<isize>> = RefCell::new(HashSet::new());
    static TARGETS: RefCell<Vec<IDropTarget>> = const { RefCell::new(Vec::new()) };
}

/// Puts our drop target on the wheel window and every window WebView2 created inside
/// it, in place of theirs. WebView2 makes its windows after start-up, so this runs each
/// time the wheel opens and only touches windows it hasn't seen.
fn register_drop_targets(app: &AppHandle, wheel: HWND) {
    unsafe extern "system" fn collect(child: HWND, list: LPARAM) -> BOOL {
        let list = unsafe { &mut *(list.0 as *mut Vec<HWND>) };
        list.push(child);
        BOOL(1)
    }
    let mut windows = vec![wheel];
    let _ = unsafe { EnumChildWindows(Some(wheel), Some(collect), LPARAM(&mut windows as *mut _ as isize)) };
    // Forget windows that are gone: Windows reuses handle numbers, and a new WebView2
    // window with an old number would otherwise be skipped and accept no drops.
    REGISTERED.with(|r| r.borrow_mut().retain(|&key| unsafe { IsWindow(Some(HWND(key as _))) }.as_bool()));
    for hwnd in windows {
        let key = hwnd.0 as isize;
        if REGISTERED.with(|r| r.borrow().contains(&key)) {
            continue;
        }
        let target: IDropTarget =
            WheelDropTarget { app: app.clone(), wheel, hover: Cell::new(Hover::NONE), has_files: Cell::new(false) }.into();
        let _ = unsafe { RevokeDragDrop(hwnd) };
        if unsafe { RegisterDragDrop(hwnd, &target) }.is_ok() {
            REGISTERED.with(|r| r.borrow_mut().insert(key));
            TARGETS.with(|t| t.borrow_mut().push(target));
        }
    }
}

#[implement(IDropTarget)]
struct WheelDropTarget {
    app: AppHandle,
    wheel: HWND,
    hover: Cell<Hover>,
    has_files: Cell<bool>,
}

impl WheelDropTarget {
    fn hover_at(&self, pt: &POINTL) -> Hover {
        let mut p = POINT { x: pt.x, y: pt.y };
        let _ = unsafe { ScreenToClient(self.wheel, &mut p) };
        let scale = unsafe { GetDpiForWindow(self.wheel) }.max(96) as f64 / 96.0;
        let (x, y) = (p.x as f64 / scale, p.y as f64 / scale);
        let n = self.app.state::<WheelState>().formats.lock().map(|f| f.len()).unwrap_or(0);
        wheel::hover_at(x - wheel::SIZE / 2.0, wheel::SIZE / 2.0 - y, n)
    }

    /// Lights up what is under the cursor and accepts the drop only over a format, and
    /// only as a copy the source allows (`allowed` is what Windows passed in).
    fn track(&self, pt: &POINTL, allowed: DROPEFFECT) -> DROPEFFECT {
        let hover = self.hover_at(pt);
        self.set_hover(hover);
        if hover.segment.is_some() && self.has_files.get() && (allowed & DROPEFFECT_COPY) == DROPEFFECT_COPY {
            DROPEFFECT_COPY
        } else {
            DROPEFFECT_NONE
        }
    }

    fn set_hover(&self, hover: Hover) {
        if self.hover.replace(hover) != hover {
            let _ = self.app.emit("wheel-hover", hover);
        }
    }
}

impl IDropTarget_Impl for WheelDropTarget_Impl {
    fn DragEnter(
        &self,
        data: Ref<'_, IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let paths = unsafe { file_paths(data) };
        self.has_files.set(!paths.is_empty());
        if !paths.is_empty() {
            trigger::files_entered(&self.app, paths);
        }
        unsafe { *effect = self.track(pt, *effect) };
        Ok(())
    }

    fn DragOver(&self, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> windows::core::Result<()> {
        unsafe { *effect = self.track(pt, *effect) };
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        self.set_hover(Hover::NONE);
        Ok(())
    }

    fn Drop(
        &self,
        data: Ref<'_, IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let hover = self.hover_at(pt);
        let allowed = unsafe { *effect };
        let paths = unsafe { file_paths(data) };
        self.set_hover(Hover::NONE);
        let converted = (allowed & DROPEFFECT_COPY) == DROPEFFECT_COPY
            && trigger::drop_on_wheel(&self.app, hover, paths);
        // Always say what happened. Left unset, Windows reports the source's own
        // allowed effects back to File Explorer, which can include "move" and so
        // delete the original.
        unsafe { *effect = if converted { DROPEFFECT_COPY } else { DROPEFFECT_NONE } };
        Ok(())
    }
}

/// Paths of the files in a drag, or nothing when it isn't a file drag.
unsafe fn file_paths(data: Ref<'_, IDataObject>) -> Vec<String> {
    let Some(data) = data.as_ref() else {
        return Vec::new();
    };
    let format = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let Ok(mut medium) = (unsafe { data.GetData(&format) }) else {
        return Vec::new();
    };
    let hdrop = HDROP(unsafe { medium.u.hGlobal.0 });
    let count = unsafe { DragQueryFileW(hdrop, u32::MAX, None) };
    let mut paths = Vec::new();
    for i in 0..count {
        let len = unsafe { DragQueryFileW(hdrop, i, None) } as usize;
        let mut buf = vec![0u16; len + 1];
        unsafe { DragQueryFileW(hdrop, i, Some(&mut buf)) };
        paths.push(OsString::from_wide(&buf[..len]).to_string_lossy().into_owned());
    }
    unsafe { ReleaseStgMedium(&mut medium) };
    paths
}
