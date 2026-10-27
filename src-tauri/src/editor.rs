//! The editing window some tools open first, Crop for now: the picture with a box to
//! drag, turned a quarter at a time, then Crop or Cancel. The page only sends numbers
//! back; which file they apply to stays here, so the page can never name one.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread;

use base64::Engine;
use serde::Serialize;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};

use crate::convert::{self, Crop};
use crate::{converters, log, notify};

const LABEL: &str = "editor";

/// The picture being edited, from opening the window to Crop or Cancel.
#[derive(Default)]
pub struct Editing(Mutex<Option<Session>>);

impl Editing {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Session>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// What the page needs to draw the picture.
#[derive(Clone, Serialize)]
pub struct Session {
    #[serde(skip)]
    path: PathBuf,
    /// The file's name, for the window.
    name: String,
    /// The picture's size upright, in pixels: the box comes back in these.
    width: u32,
    height: u32,
    /// A JPEG of it, at most 1400 pixels across, as a data: URL.
    preview: String,
}

/// Opens the editor on one picture. Making its preview takes a moment, so that happens
/// on a thread of its own and the window opens once it is ready.
pub fn open(app: &AppHandle, paths: Vec<String>) {
    let [path] = paths.as_slice() else {
        notify(app, "Crop takes one picture at a time", "Drag a single picture onto Crop.");
        return;
    };
    let (app, path) = (app.clone(), PathBuf::from(path));
    thread::spawn(move || match convert::preview(&path) {
        Ok((width, height, jpeg)) => {
            let name = file_name(&path);
            let preview = format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(jpeg));
            *app.state::<Editing>().lock() = Some(Session { path, name: name.clone(), width, height, preview });
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || show_window(&handle, &format!("Crop: {name}")));
        }
        Err(e) => {
            log::line(&format!("couldn't open {} in the editor: {e}", file_name(&path)));
            notify(&app, "Couldn't open the picture", &converters::offer(&app, e));
        }
    });
}

/// Shows the editor window, or reloads it for the new picture if it is open already.
fn show_window(app: &AppHandle, title: &str) {
    let window = match app.get_webview_window(LABEL) {
        Some(window) => {
            let _ = window.eval("location.reload()");
            window
        }
        None => match WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("editor.html".into()))
            .title(title)
            .inner_size(780.0, 620.0)
            .min_inner_size(560.0, 460.0)
            .center()
            .build()
        {
            Ok(window) => window,
            Err(e) => {
                log::line(&format!("couldn't open the editor: {e}"));
                notify(app, "Couldn't open the editor", &e.to_string());
                return;
            }
        },
    };
    let _ = window.set_title(title);
    let _ = window.show();
    // A menu bar app is never the active one; without this the window opens behind.
    #[cfg(target_os = "macos")]
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        #[allow(deprecated)]
        objc2_app_kit::NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    }
    let _ = window.set_focus();
}

fn close_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.close();
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The page asks what to show when it loads.
#[tauri::command]
pub fn editor_session(editing: State<Editing>) -> Option<Session> {
    editing.lock().clone()
}

/// Crop: checks the box against the picture and queues the job.
#[tauri::command]
pub fn editor_apply(app: AppHandle, editing: State<Editing>, crop: Crop) -> Result<(), String> {
    let mut session = editing.lock();
    let Some(current) = session.as_ref() else {
        return Err("nothing to crop: drag the picture onto Crop again".into());
    };
    crop.check(current.width, current.height)?;
    let path = current.path.to_string_lossy().into_owned();
    *session = None;
    drop(session);
    close_window(&app);
    crate::enqueue_crop(&app, path, crop);
    Ok(())
}

#[tauri::command]
pub fn editor_cancel(app: AppHandle, editing: State<Editing>) {
    *editing.lock() = None;
    close_window(&app);
}
