//! Shows converted files selected in the system's file manager: after a click on the
//! notification that reported them, or on the menu's "Last: …" line.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread;

use tauri::{AppHandle, Manager};

/// How many finished drops a click on their notification can still show.
const REMEMBERED: usize = 20;

/// What the recent drops produced, by the notification that reported each one.
#[derive(Default)]
pub struct Results(Mutex<Recent>);

#[derive(Default)]
struct Recent {
    last: Vec<PathBuf>,
    notified: VecDeque<(i32, Vec<PathBuf>)>,
}

impl Results {
    /// Remembers what a finished drop produced (nothing when every file failed), and
    /// the notification that reported it.
    pub fn finished(&self, notification: i32, files: Vec<PathBuf>) {
        let mut recent = self.0.lock().unwrap_or_else(|e| e.into_inner());
        recent.last = files.clone();
        if files.is_empty() {
            return;
        }
        if recent.notified.len() == REMEMBERED {
            recent.notified.pop_front();
        }
        recent.notified.push_back((notification, files));
    }

    fn last(&self) -> Vec<PathBuf> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).last.clone()
    }

    fn notified(&self, notification: i32) -> Option<Vec<PathBuf>> {
        let recent = self.0.lock().unwrap_or_else(|e| e.into_inner());
        recent.notified.iter().find(|(id, _)| *id == notification).map(|(_, files)| files.clone())
    }
}

/// A click on a notification: shows the files it reported, if it reported any.
pub fn show_notified(app: &AppHandle, notification: i32) {
    if let Some(files) = app.state::<Results>().notified(notification) {
        show(app, files);
    }
}

/// The menu's last-result line: shows what the last drop produced.
pub fn show_last(app: &AppHandle) {
    let files = app.state::<Results>().last();
    if !files.is_empty() {
        show(app, files);
    }
}

/// Opens the folder that holds `files` with them selected, on a thread of its own: a
/// file manager that is slow to answer must not hold up the menu. Files moved or
/// deleted since are left out.
pub fn show(app: &AppHandle, files: Vec<PathBuf>) {
    let app = app.clone();
    thread::spawn(move || {
        let there: Vec<PathBuf> = files.into_iter().filter(|f| f.exists()).collect();
        if there.is_empty() {
            crate::notify(&app, "Couldn't show the file", "It was moved or deleted since.");
            return;
        }
        if let Err(e) = select(&there) {
            crate::log::line(&format!("couldn't show {}: {e}", there[0].display()));
            crate::notify(&app, "Couldn't open the folder", &e);
        }
    });
}

/// Finder selects every file, in one window.
#[cfg(target_os = "macos")]
fn select(files: &[PathBuf]) -> Result<(), String> {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSArray, NSString, NSURL};

    let urls: Vec<_> = files
        .iter()
        .filter_map(|f| f.to_str())
        .map(|f| NSURL::fileURLWithPath(&NSString::from_str(f)))
        .collect();
    if urls.is_empty() {
        return Err("the file name is not valid text".into());
    }
    NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&urls));
    Ok(())
}

/// Through the shell, not explorer.exe's own parsing of a "/select," argument, which
/// trips over commas and quotes in file names. Explorer selects only the first file.
#[cfg(windows)]
fn select(files: &[PathBuf]) -> Result<(), String> {
    use windows::core::HSTRING;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{ILCreateFromPathW, ILFree, SHOpenFolderAndSelectItems};

    unsafe {
        // SAFETY: this thread is show()'s own, so COM is set up here for this call
        // alone and taken down after it. The item list is freed once, after its use.
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let item = ILCreateFromPathW(&HSTRING::from(files[0].as_path()));
        let result = if item.is_null() {
            Err("Windows can't find the file".to_string())
        } else {
            let shown = SHOpenFolderAndSelectItems(item, None, 0).map_err(|e| e.to_string());
            ILFree(Some(item));
            shown
        };
        if com.is_ok() {
            CoUninitialize();
        }
        result
    }
}

/// The file manager selects the files when it takes part in the FileManager1 D-Bus
/// interface (Files, Dolphin, Nemo, Caja and others). Otherwise the folder opens with
/// nothing selected.
#[cfg(target_os = "linux")]
fn select(files: &[PathBuf]) -> Result<(), String> {
    let uris: Vec<String> = files
        .iter()
        .filter_map(|f| tauri::Url::from_file_path(f).ok())
        .map(String::from)
        .collect();
    let asked = zbus::blocking::Connection::session().and_then(|bus| {
        bus.call_method(
            Some("org.freedesktop.FileManager1"),
            "/org/freedesktop/FileManager1",
            Some("org.freedesktop.FileManager1"),
            "ShowItems",
            &(uris, ""),
        )
    });
    if asked.is_ok() {
        return Ok(());
    }
    let folder = files[0].parent().unwrap_or(&files[0]);
    std::process::Command::new("xdg-open")
        .arg(folder)
        .spawn()
        .map(drop)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_finds_the_files_its_notification_reported() {
        let results = Results::default();
        results.finished(1, vec!["a.png".into()]);
        results.finished(2, vec!["b.png".into(), "c.png".into()]);
        assert_eq!(results.notified(1), Some(vec![PathBuf::from("a.png")]));
        assert_eq!(results.notified(2), Some(vec![PathBuf::from("b.png"), PathBuf::from("c.png")]));
        assert_eq!(results.notified(3), None);
        assert_eq!(results.last(), vec![PathBuf::from("b.png"), PathBuf::from("c.png")]);
    }

    #[test]
    fn a_drop_where_every_file_failed_leaves_nothing_to_show() {
        let results = Results::default();
        results.finished(1, vec!["a.png".into()]);
        results.finished(2, Vec::new());
        assert!(results.last().is_empty());
        assert_eq!(results.notified(2), None);
        // The earlier notification still shows its own file.
        assert_eq!(results.notified(1), Some(vec![PathBuf::from("a.png")]));
    }

    #[test]
    fn only_the_recent_notifications_are_remembered() {
        let results = Results::default();
        for id in 0..(REMEMBERED as i32 + 5) {
            results.finished(id, vec![format!("{id}.png").into()]);
        }
        assert_eq!(results.notified(4), None);
        assert_eq!(results.notified(5), Some(vec![PathBuf::from("5.png")]));
        assert_eq!(results.0.lock().unwrap().notified.len(), REMEMBERED);
    }
}
