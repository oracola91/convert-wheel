//! Float on Windows and Linux: the dragged files on a small card that floats above
//! every window, to drag out again later into any app or folder, as on macOS
//! (float_card.rs). The card is a small page, float.html. Its icon and name are where
//! the files are taken out: on Windows the page asks for the system's own file drag
//! (SHDoDragDrop, so Explorer and every app see real files); on Linux the page's own
//! drag carries them as a uri-list, which file managers and apps take. The rest of the
//! card moves it. Its close button puts it away, and so does a drag that moved the files
//! somewhere else. macOS builds this too, for the tests, but uses float_card.rs.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::log;

const WIDTH: f64 = 250.0;
const HEIGHT: f64 = 64.0;
/// Each new card sits a little lower and to the right of the one before.
const CASCADE: f64 = 22.0;
/// How far from the pointer a new card opens.
const OFFSET: f64 = 16.0;

static NEXT: AtomicUsize = AtomicUsize::new(1);
/// The cards on screen, by window label, with the files each one holds.
static CARDS: Mutex<Vec<(String, Vec<PathBuf>)>> = Mutex::new(Vec::new());

/// What the page shows and, on Linux, what its drag carries.
#[derive(Serialize, Debug, PartialEq)]
pub struct Card {
    title: String,
    detail: String,
    /// Windows: the page asks for the system's drag instead of starting its own.
    native: bool,
    uris: Vec<String>,
    paths: Vec<String>,
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn open(app: &AppHandle, files: Vec<PathBuf>) {
    let files: Vec<PathBuf> = files.into_iter().filter(|f| f.exists()).collect();
    if files.is_empty() {
        return;
    }
    let label = format!("float-{}", NEXT.fetch_add(1, Ordering::Relaxed));
    let shown = {
        let mut cards = CARDS.lock().unwrap_or_else(|e| e.into_inner());
        cards.push((label.clone(), files));
        cards.len()
    };
    let (x, y) = place(app, shown);
    let built = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("float.html".into()))
        .title("Float")
        .inner_size(WIDTH, HEIGHT)
        .position(x, y)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible_on_all_workspaces(true)
        .focused(false)
        .build();
    if let Err(e) = built {
        log::line(&format!("couldn't open a Float card: {e}"));
        forget(&label);
    }
}

/// Below and right of the pointer, after the cards already there, inside the screen
/// the pointer is on. In logical pixels, as the window builder takes them.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn place(app: &AppHandle, cards: usize) -> (f64, f64) {
    let Ok(pointer) = app.cursor_position() else {
        return (100.0, 100.0);
    };
    let monitor = app.monitor_from_point(pointer.x, pointer.y).ok().flatten();
    let scale = monitor.as_ref().map_or(1.0, |m| m.scale_factor());
    let step = CASCADE * cards.saturating_sub(1) as f64;
    let (mut x, mut y) = (pointer.x / scale + OFFSET + step, pointer.y / scale + OFFSET + step);
    if let Some(m) = monitor {
        let area = m.work_area();
        let (left, top) = (area.position.x as f64 / scale, area.position.y as f64 / scale);
        let (right, bottom) = (left + area.size.width as f64 / scale, top + area.size.height as f64 / scale);
        x = x.min(right - WIDTH - 8.0).max(left + 8.0);
        y = y.min(bottom - HEIGHT - 8.0).max(top + 8.0);
    }
    (x, y)
}

fn files_of(label: &str) -> Option<Vec<PathBuf>> {
    let cards = CARDS.lock().unwrap_or_else(|e| e.into_inner());
    cards.iter().find(|(l, _)| l == label).map(|(_, f)| f.clone())
}

fn forget(label: &str) {
    CARDS.lock().unwrap_or_else(|e| e.into_inner()).retain(|(l, _)| l != label);
}

fn describe(files: &[PathBuf]) -> (String, String) {
    match files {
        [one] => {
            let name = one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| one.display().to_string());
            (name, "Drag it out when you need it".to_string())
        }
        many => (format!("{} files", many.len()), "Drag them out when you need them".to_string()),
    }
}

/// A file:// address for a uri-list: every byte outside the unreserved set and `/` is
/// %-escaped, so spaces, `#` and non-ASCII names survive.
fn file_uri(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file://");
    if !text.starts_with('/') {
        out.push('/');
    }
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => out.push(b as char),
            b':' if out.len() == 9 => out.push(':'), // the drive letter's colon on Windows
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn card_for(files: &[PathBuf]) -> Card {
    let (title, detail) = describe(files);
    Card {
        title,
        detail,
        native: cfg!(windows),
        uris: files.iter().map(|f| file_uri(f)).collect(),
        paths: files.iter().map(|f| f.display().to_string()).collect(),
    }
}

/// The page asks what its card holds.
#[tauri::command]
pub fn float_card(window: WebviewWindow) -> Option<Card> {
    files_of(window.label()).map(|f| card_for(&f))
}

/// The close button.
#[tauri::command]
pub fn float_close(window: WebviewWindow) {
    forget(window.label());
    let _ = window.close();
}

/// The empty part of the card was pressed: move the window with the pointer.
#[tauri::command]
pub fn float_move(window: WebviewWindow) {
    let _ = window.start_dragging();
}

/// Windows: the files' own drag, started while the button is still down. A sync
/// command, so it runs on the main thread, as the system's drag loop must.
#[tauri::command]
pub fn float_drag(window: WebviewWindow) -> Result<(), String> {
    let files = files_of(window.label()).ok_or("this card holds nothing")?;
    drag_out(&window, &files)?;
    after_drag(&window, &files);
    Ok(())
}

/// Linux: the page's own drag ended.
#[tauri::command]
pub fn float_dropped(window: WebviewWindow) {
    if let Some(files) = files_of(window.label()) {
        after_drag(&window, &files);
    }
}

/// A drag that moved the files elsewhere leaves nothing to hold: the card goes.
fn after_drag(window: &WebviewWindow, files: &[PathBuf]) {
    if files.iter().any(|f| !f.exists()) {
        forget(window.label());
        let _ = window.close();
    }
}

#[cfg(windows)]
fn drag_out(window: &WebviewWindow, files: &[PathBuf]) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::System::Com::{CoTaskMemFree, IDataObject};
    use windows::Win32::System::Ole::{IDropSource, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE};
    use windows::Win32::UI::Shell::Common::ITEMIDLIST;
    use windows::Win32::UI::Shell::{BHID_DataObject, IShellItemArray, SHCreateShellItemArrayFromIDLists, SHDoDragDrop, SHParseDisplayName};

    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    let mut pidls: Vec<*mut ITEMIDLIST> = Vec::new();
    let result = (|| -> windows::core::Result<()> {
        for f in files {
            let wide: Vec<u16> = f.as_os_str().encode_wide().chain(Some(0)).collect();
            let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
            unsafe { SHParseDisplayName(PCWSTR(wide.as_ptr()), None, &mut pidl, 0, None)? };
            pidls.push(pidl);
        }
        let list: Vec<*const ITEMIDLIST> = pidls.iter().map(|p| *p as *const ITEMIDLIST).collect();
        let items: IShellItemArray = unsafe { SHCreateShellItemArrayFromIDLists(&list)? };
        let data: IDataObject = unsafe { items.BindToHandler(None, &BHID_DataObject)? };
        let mut effect = DROPEFFECT(0);
        // No drop source of our own: the shell makes its default one.
        unsafe { SHDoDragDrop(Some(hwnd), &data, None::<&IDropSource>, DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK, &mut effect) }
    })();
    for p in pidls {
        unsafe { CoTaskMemFree(Some(p as *const _)) };
    }
    result.map_err(|e| {
        log::line(&format!("Float drag failed: {e}"));
        e.to_string()
    })
}

#[cfg(not(windows))]
fn drag_out(_window: &WebviewWindow, _files: &[PathBuf]) -> Result<(), String> {
    Err("the page drags the files itself here".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_file_shows_its_name_several_a_count() {
        assert_eq!(describe(&[PathBuf::from("/tmp/a b.pdf")]).0, "a b.pdf");
        assert_eq!(describe(&[PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")]).0, "2 files");
    }

    #[test]
    fn uris_escape_what_a_uri_list_cannot_hold() {
        assert_eq!(file_uri(Path::new("/home/me/My file #1.pdf")), "file:///home/me/My%20file%20%231.pdf");
        assert_eq!(file_uri(Path::new("/tmp/Résumé.txt")), "file:///tmp/R%C3%A9sum%C3%A9.txt");
        assert_eq!(file_uri(Path::new(r"C:\Users\me\a b.txt")), "file:///C:/Users/me/a%20b.txt");
    }

    #[test]
    fn a_card_is_forgotten_once_put_away() {
        CARDS.lock().unwrap().push(("float-test".into(), vec![PathBuf::from("/tmp/x")]));
        assert!(files_of("float-test").is_some());
        forget("float-test");
        assert!(files_of("float-test").is_none());
    }
}
