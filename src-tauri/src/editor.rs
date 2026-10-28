//! The editing window some tools open first: Crop (the picture with a box to drag,
//! turned a quarter at a time), Trim (a sound's or video's loudness and frames, with a
//! start and an end to drag) and Compress to a Size (a size to type or pick). The page only sends numbers back; which file they
//! apply to stays here, so the page can never name one.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread;

use base64::Engine;
use serde::Serialize;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};

use crate::convert::{self, Crop, Edit, Trim};
use crate::{converters, log, notify};

const LABEL: &str = "editor";

/// The file being edited, from opening the window to the tool's button or Cancel.
#[derive(Default)]
pub struct Editing(Mutex<Option<Session>>);

impl Editing {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Session>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// What the page needs: the file's name and what to draw.
#[derive(Clone, Serialize)]
pub struct Session {
    #[serde(skip)]
    path: PathBuf,
    name: String,
    #[serde(flatten)]
    view: View,
}

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum View {
    /// The picture's size upright, in pixels (the box comes back in these), and a JPEG
    /// of it, at most 1400 pixels across, as a data: URL.
    Crop { width: u32, height: u32, preview: String },
    /// The length (the times come back in milliseconds), the loudness along it, and for
    /// a video eight frames; the sound as a small MP3 to play, when not too long.
    Trim { length_ms: u64, peaks: Vec<u8>, strip: Option<String>, sound: Option<String> },
    /// The file's size, what it is ("picture", "video", "sound"), for sound and video
    /// the least it can be compressed to, and for a picture the sizes under which it
    /// will probably get smaller, and small and blurry, to warn about.
    Size { bytes: u64, what: &'static str, least: Option<u64>, smaller_below: Option<u64>, blurry_below: Option<u64> },
}

impl View {
    fn page(&self) -> &'static str {
        match self {
            View::Crop { .. } => "editor.html",
            View::Trim { .. } => "trim.html",
            View::Size { .. } => "size.html",
        }
    }

    /// The window's size and smallest size: a short form for Compress to a Size.
    fn window(&self) -> ((f64, f64), (f64, f64)) {
        match self {
            View::Size { .. } => ((480.0, 370.0), (420.0, 330.0)),
            _ => ((780.0, 620.0), (560.0, 460.0)),
        }
    }

    fn title(&self, name: &str) -> String {
        match self {
            View::Crop { .. } => format!("Crop: {name}"),
            View::Trim { .. } => format!("Trim: {name}"),
            View::Size { .. } => format!("Compress to a size: {name}"),
        }
    }
}

fn data_url(mime: &str, bytes: &[u8]) -> String {
    format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// Opens the editor for `tool` on one file. Reading the file for it takes a moment, so
/// that happens on a thread of its own and the window opens once it is ready.
pub fn open(app: &AppHandle, tool: &str, paths: Vec<String>) {
    let [path] = paths.as_slice() else {
        notify(app, "One file at a time", "Crop, Trim and Compress to a Size open one file: drag just one.");
        return;
    };
    let (app, path, tool) = (app.clone(), PathBuf::from(path), tool.to_string());
    thread::spawn(move || {
        let view = if tool == "tool:target" {
            size_view(&path)
        } else if tool == "tool:crop" {
            convert::preview(&path)
                .map(|(width, height, jpeg)| View::Crop { width, height, preview: data_url("image/jpeg", &jpeg) })
        } else {
            convert::media_preview(&path).map(|m| View::Trim {
                length_ms: m.length_ms,
                peaks: m.peaks,
                strip: m.strip.map(|jpeg| data_url("image/jpeg", &jpeg)),
                sound: m.sound.map(|mp3| data_url("audio/mpeg", &mp3)),
            })
        };
        match view {
            Ok(view) => {
                let name = file_name(&path);
                let (page, title, size) = (view.page(), view.title(&name), view.window());
                *app.state::<Editing>().lock() = Some(Session { path, name, view });
                let handle = app.clone();
                let _ = app.run_on_main_thread(move || show_window(&handle, page, &title, size));
            }
            Err(e) => {
                log::line(&format!("couldn't open {} in the editor: {e}", file_name(&path)));
                notify(&app, "Couldn't open the file", &converters::offer(&app, e));
            }
        }
    });
}

/// Shows the editor window on `page`, or moves the open one there for the new file.
fn show_window(app: &AppHandle, page: &str, title: &str, ((width, height), (min_width, min_height)): ((f64, f64), (f64, f64))) {
    let window = match app.get_webview_window(LABEL) {
        Some(window) => {
            let _ = window.eval(format!("location.replace('{page}')"));
            let _ = window.set_min_size(Some(tauri::LogicalSize::new(min_width, min_height)));
            let _ = window.set_size(tauri::LogicalSize::new(width, height));
            window
        }
        None => match WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App(page.into()))
            .title(title)
            .inner_size(width, height)
            .min_inner_size(min_width, min_height)
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

/// What Compress to a Size shows: the file's size and kind, and how small it can go.
fn size_view(path: &Path) -> Result<View, String> {
    let bytes = std::fs::metadata(path).map_err(|e| format!("can't read the file: {e}"))?.len();
    let ext = crate::formats::extension(path).unwrap_or_default();
    let what = match crate::formats::kind_of(&ext) {
        Some(crate::formats::Kind::Video) => "video",
        Some(crate::formats::Kind::Audio) => "sound",
        _ => "picture",
    };
    let warnings = if what == "picture" { convert::picture_warnings(path) } else { None };
    Ok(View::Size {
        bytes,
        what,
        least: convert::smallest_size(path),
        smaller_below: warnings.map(|w| w.0),
        blurry_below: warnings.map(|w| w.1),
    })
}

fn close_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.close();
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Takes the session if it is for the tool `check` accepts, then queues the job.
fn finish(app: &AppHandle, editing: &Editing, tool: &str, check: impl FnOnce(&View) -> Result<Edit, String>) -> Result<(), String> {
    let mut session = editing.lock();
    let Some(current) = session.as_ref() else {
        return Err("nothing to do: drag the file onto the wheel again".into());
    };
    let edit = check(&current.view)?;
    let path = current.path.to_string_lossy().into_owned();
    *session = None;
    drop(session);
    close_window(app);
    crate::enqueue_edit(app, path, tool, edit);
    Ok(())
}

/// The page asks what to show when it loads.
#[tauri::command]
pub fn editor_session(editing: State<Editing>) -> Option<Session> {
    editing.lock().clone()
}

/// Crop: checks the box against the picture and queues the job.
#[tauri::command]
pub fn editor_apply(app: AppHandle, editing: State<Editing>, crop: Crop) -> Result<(), String> {
    finish(&app, &editing, "tool:crop", |view| match view {
        View::Crop { width, height, .. } => crop.check(*width, *height).map(|()| Edit::Crop(crop)),
        _ => Err("this window doesn't crop".into()),
    })
}

/// Trim: checks the times against the file's length and queues the job.
#[tauri::command]
pub fn editor_trim(app: AppHandle, editing: State<Editing>, trim: Trim) -> Result<(), String> {
    finish(&app, &editing, "tool:trim", |view| match view {
        View::Trim { length_ms, .. } => trim.check(*length_ms).map(|()| Edit::Trim(trim)),
        _ => Err("this window doesn't trim".into()),
    })
}

/// Compress to a Size: a size under the file's own, of at least a kilobyte.
#[tauri::command]
pub fn editor_size(app: AppHandle, editing: State<Editing>, bytes: u64) -> Result<(), String> {
    finish(&app, &editing, "tool:target", |view| match view {
        View::Size { bytes: own, .. } if bytes >= 1_000 && bytes < *own => Ok(Edit::Size(bytes)),
        View::Size { .. } => Err("pick a size under the file's own, of at least 1 KB".into()),
        _ => Err("this window doesn't compress".into()),
    })
}

#[tauri::command]
pub fn editor_cancel(app: AppHandle, editing: State<Editing>) {
    *editing.lock() = None;
    close_window(&app);
}
