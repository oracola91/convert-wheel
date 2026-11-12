//! The editing window some tools open first: Crop (the picture, or a frame of a video,
//! with a box to drag, turned a quarter at a time), Adjust (sliders for light and
//! colour), Mark Up (drawing and blacking out on the picture; Black Out uses it on a
//! frame of a video, with black boxes only), Trim (a sound's or
//! video's loudness and frames, with a start and an end to drag; Bleep uses it too,
//! for the part to bleep), Speed (a speed to pick), Split for sound and video (how many
//! parts, or how long each), Compress to a Size (a size to type or pick), Reorder (a
//! PDF's pages, to drag into a new order) and Info (what a file says about itself, and
//! a sound's or PDF's tags to change). The page only sends numbers back (and Mark Up's
//! words, Info's tags); which file they apply to stays here, so the page can never name
//! one.

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
    /// Compress to %: the other files dropped with the first. Empty for the others.
    #[serde(skip)]
    more: Vec<PathBuf>,
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
    /// The same picture, for Adjust's sliders.
    Adjust { preview: String },
    /// The same picture and its size, for Mark Up's drawing (sent back in its pixels).
    /// With `video`, a frame of a video for Black Out: the window offers only black
    /// boxes, which cover the same place for the whole video.
    Markup { width: u32, height: u32, preview: String, video: bool },
    /// The length (the times come back in milliseconds), the loudness along it, and for
    /// a video eight frames; the sound as a small MP3 to play, when not too long.
    /// With `bleep`, the same window picks the part Bleep turns into a tone.
    Trim { length_ms: u64, peaks: Vec<u8>, strip: Option<String>, sound: Option<String>, bleep: bool },
    /// The file's size, what it is ("picture", "video", "sound"), for sound and video
    /// the least it can be compressed to, and for a picture the sizes under which it
    /// will probably get smaller, and small and blurry, to warn about.
    Size { bytes: u64, what: &'static str, least: Option<u64>, smaller_below: Option<u64>, blurry_below: Option<u64> },
    /// How long it plays, what it is ("video", "sound"), and the speeds offered.
    Speed { length_ms: u64, what: &'static str, speeds: &'static [u16] },
    /// How long it plays and what it is, to split into parts.
    Parts { length_ms: u64, what: &'static str },
    /// A small PNG of each page of a PDF, in page order, as data: URLs. The new order
    /// comes back as page numbers from 1.
    Order { pages: Vec<String> },
    /// What the file is, what it says (label, value) and the tags that can be changed.
    Info { what: &'static str, facts: Vec<(String, String)>, fields: Vec<convert::info::Field> },
    /// Resize: the picture's or video's size upright, what it is ("picture", "video"),
    /// and the smallest and largest longest side the app takes.
    Resize { width: u32, height: u32, what: &'static str, min: u32, max: u32 },
    /// Password: the longest password the app takes.
    Lock { max: usize },
    /// Snapshot: how long the video plays; frames come one at a time (editor_frame).
    Snapshot { length_ms: u64 },
    /// Compress to %: how many files, their size together, and the shares taken.
    Percent { count: usize, bytes: u64, min: u8, max: u8 },
}

impl View {
    fn page(&self) -> &'static str {
        match self {
            View::Crop { .. } => "editor.html",
            View::Adjust { .. } => "adjust.html",
            View::Markup { .. } => "markup.html",
            View::Trim { .. } => "trim.html",
            View::Size { .. } => "size.html",
            View::Speed { .. } => "speed.html",
            View::Parts { .. } => "parts.html",
            View::Order { .. } => "order.html",
            View::Info { .. } => "info.html",
            View::Resize { .. } => "resize.html",
            View::Lock { .. } => "lock.html",
            View::Snapshot { .. } => "snapshot.html",
            View::Percent { .. } => "percent.html",
        }
    }

    /// The window's size and smallest size: a short form for Compress to a Size.
    fn window(&self) -> ((f64, f64), (f64, f64)) {
        match self {
            View::Size { .. } => ((480.0, 370.0), (420.0, 330.0)),
            View::Speed { .. } => ((480.0, 300.0), (420.0, 280.0)),
            View::Resize { .. } => ((480.0, 340.0), (420.0, 320.0)),
            View::Lock { .. } => ((440.0, 300.0), (400.0, 280.0)),
            View::Snapshot { .. } => ((720.0, 560.0), (520.0, 440.0)),
            View::Percent { .. } => ((480.0, 320.0), (420.0, 300.0)),
            View::Parts { .. } => ((480.0, 360.0), (420.0, 330.0)),
            View::Info { .. } => ((480.0, 560.0), (420.0, 360.0)),
            // Room for all of Mark Up's tools on one row.
            View::Markup { .. } => ((880.0, 660.0), (640.0, 480.0)),
            _ => ((780.0, 620.0), (560.0, 460.0)),
        }
    }

    fn title(&self, name: &str) -> String {
        match self {
            View::Crop { .. } => format!("Crop: {name}"),
            View::Adjust { .. } => format!("Adjust: {name}"),
            View::Markup { video: true, .. } => format!("Black out: {name}"),
            View::Markup { .. } => format!("Mark up: {name}"),
            View::Trim { bleep: true, .. } => format!("Bleep: {name}"),
            View::Trim { .. } => format!("Trim: {name}"),
            View::Size { .. } => format!("Compress to a size: {name}"),
            View::Speed { .. } => format!("Speed: {name}"),
            View::Parts { .. } => format!("Split: {name}"),
            View::Order { .. } => format!("Reorder: {name}"),
            View::Info { .. } => format!("Info: {name}"),
            View::Resize { .. } => format!("Resize: {name}"),
            View::Lock { .. } => format!("Password: {name}"),
            View::Snapshot { .. } => format!("Snapshot: {name}"),
            View::Percent { .. } => format!("Compress to %: {name}"),
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
        notify(app, "One file at a time", "Crop, Adjust, Mark Up, Black Out, Trim, Bleep, Speed, Split, Reorder, Info, Resize, Password, Snapshot and Compress to a Size open one file: drag just one.");
        return;
    };
    let (app, path, tool) = (app.clone(), PathBuf::from(path), tool.to_string());
    thread::spawn(move || {
        let view = if tool == "tool:target" {
            size_view(&path)
        } else if tool == "tool:speed" {
            play_length(&path).map(|(length_ms, what)| View::Speed { length_ms, what, speeds: convert::SPEEDS })
        } else if tool == "tool:parts" {
            play_length(&path).map(|(length_ms, what)| View::Parts { length_ms, what })
        } else if tool == "tool:order" {
            convert::page_pictures(&path)
                .map(|pngs| View::Order { pages: pngs.iter().map(|png| data_url("image/png", png)).collect() })
        } else if tool == "tool:resize" {
            resize_view(&path)
        } else if tool == "tool:snapshot" {
            convert::length_ms(&path).map(|length_ms| View::Snapshot { length_ms })
        } else if tool == "tool:lock" {
            Ok(View::Lock { max: convert::MAX_PASSWORD })
        } else if tool == "tool:info" {
            convert::info::read(&path).map(|i| View::Info { what: i.what, facts: i.facts, fields: i.fields })
        } else if tool == "tool:markup" {
            convert::preview(&path)
                .map(|(width, height, jpeg)| View::Markup { width, height, preview: data_url("image/jpeg", &jpeg), video: false })
        } else if tool == "tool:blackout" {
            convert::blackout_preview(&path)
                .map(|(width, height, jpeg)| View::Markup { width, height, preview: data_url("image/jpeg", &jpeg), video: true })
        } else if tool == "tool:adjust" {
            convert::preview(&path).map(|(_, _, jpeg)| View::Adjust { preview: data_url("image/jpeg", &jpeg) })
        } else if tool == "tool:crop" {
            // A video is cropped on one of its frames.
            let ext = crate::formats::extension(&path).unwrap_or_default();
            let video = crate::formats::kind_of(&ext) == Some(crate::formats::Kind::Video);
            if video { convert::video_preview(&path) } else { convert::preview(&path) }
                .map(|(width, height, jpeg)| View::Crop { width, height, preview: data_url("image/jpeg", &jpeg) })
        } else {
            convert::media_preview(&path).map(|m| View::Trim {
                length_ms: m.length_ms,
                peaks: m.peaks,
                strip: m.strip.map(|jpeg| data_url("image/jpeg", &jpeg)),
                sound: m.sound.map(|mp3| data_url("audio/mpeg", &mp3)),
                bleep: tool == "tool:bleep",
            })
        };
        match view {
            Ok(view) => {
                let name = file_name(&path);
                let (page, title, size) = (view.page(), view.title(&name), view.window());
                *app.state::<Editing>().lock() = Some(Session { path, more: Vec::new(), name, view });
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

/// Compress to %: one window for every file dropped (two or more).
pub fn open_percent(app: &AppHandle, paths: Vec<String>) {
    let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    let bytes = paths.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum();
    let [first, rest @ ..] = paths.as_slice() else {
        return;
    };
    let view = View::Percent { count: paths.len(), bytes, min: convert::PERCENT_MIN, max: convert::PERCENT_MAX };
    let name = format!("{} files", paths.len());
    let (page, title, size) = (view.page(), view.title(&name), view.window());
    *app.state::<Editing>().lock() = Some(Session { path: first.clone(), more: rest.to_vec(), name, view });
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || show_window(&handle, page, &title, size));
}

/// Compress to %: a share within the limits, for all the files at once.
#[tauri::command]
pub fn editor_percent(app: AppHandle, editing: State<Editing>, percent: u8) -> Result<(), String> {
    let mut session = editing.lock();
    let Some(current) = session.as_ref() else {
        return Err("nothing to do: drag the files onto the wheel again".into());
    };
    let View::Percent { min, max, .. } = current.view else {
        return Err("this window doesn't compress to a share".into());
    };
    if !(min..=max).contains(&percent) {
        return Err(format!("pick a share from {min}% to {max}%"));
    }
    let paths = std::iter::once(&current.path).chain(&current.more).map(|p| p.to_string_lossy().into_owned()).collect();
    *session = None;
    drop(session);
    close_window(&app);
    crate::enqueue_edits(&app, paths, "tool:percent", Edit::Percent(percent));
    Ok(())
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
        Some(crate::formats::Kind::Pdf) => "PDF",
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

/// What Resize shows: the size upright (a video's from one of its frames), and the
/// limits for the longest side.
fn resize_view(path: &Path) -> Result<View, String> {
    let ext = crate::formats::extension(path).unwrap_or_default();
    let video = crate::formats::kind_of(&ext) == Some(crate::formats::Kind::Video);
    let (width, height, _) = if video { convert::video_preview(path)? } else { convert::preview(path)? };
    let (what, max) = if video { ("video", convert::RESIZE_VIDEO_MAX) } else { ("picture", convert::RESIZE_PICTURE_MAX) };
    Ok(View::Resize { width, height, what, min: convert::RESIZE_MIN, max })
}

/// For Speed and Split: how long the file plays, and whether it is a video or a sound.
fn play_length(path: &Path) -> Result<(u64, &'static str), String> {
    let ext = crate::formats::extension(path).unwrap_or_default();
    let what = if crate::formats::kind_of(&ext) == Some(crate::formats::Kind::Video) { "video" } else { "sound" };
    Ok((convert::length_ms(path)?, what))
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
        View::Trim { length_ms, bleep: false, .. } => trim.check(*length_ms).map(|()| Edit::Trim(trim)),
        _ => Err("this window doesn't trim".into()),
    })
}

/// Bleep: the part to turn into a tone, checked like Trim's part to keep.
#[tauri::command]
pub fn editor_bleep(app: AppHandle, editing: State<Editing>, part: Trim) -> Result<(), String> {
    finish(&app, &editing, "tool:bleep", |view| match view {
        View::Trim { length_ms, bleep: true, .. } => part.check(*length_ms).map(|()| Edit::Trim(part)),
        _ => Err("this window doesn't bleep".into()),
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

/// Resize: a longest side within the limits, and not the size it already has.
#[tauri::command]
pub fn editor_resize(app: AppHandle, editing: State<Editing>, side: u32) -> Result<(), String> {
    finish(&app, &editing, "tool:resize", |view| match view {
        View::Resize { width, height, min, max, .. } if (*min..=*max).contains(&side) && side != (*width).max(*height) => {
            Ok(Edit::Resize(side))
        }
        View::Resize { width, height, .. } if side == (*width).max(*height) => Err("that is the size it has already".into()),
        View::Resize { min, max, .. } => Err(format!("pick a size from {min} to {max} pixels")),
        _ => Err("this window doesn't resize".into()),
    })
}

/// Password: one the app takes (convert::Password::new says why not).
#[tauri::command]
pub fn editor_lock(app: AppHandle, editing: State<Editing>, password: String) -> Result<(), String> {
    finish(&app, &editing, "tool:lock", |view| match view {
        View::Lock { .. } => convert::Password::new(password).map(Edit::Password),
        _ => Err("this window doesn't add a password".into()),
    })
}

/// Speed: one of the speeds the window offers.
#[tauri::command]
pub fn editor_speed(app: AppHandle, editing: State<Editing>, percent: u16) -> Result<(), String> {
    finish(&app, &editing, "tool:speed", |view| match view {
        View::Speed { speeds, .. } if speeds.contains(&percent) => Ok(Edit::Speed(percent)),
        View::Speed { .. } => Err("pick one of the speeds offered".into()),
        _ => Err("this window doesn't change speed".into()),
    })
}

/// Split for sound and video: parts of this many milliseconds, if that makes from two
/// to the most parts it takes.
#[tauri::command]
pub fn editor_parts(app: AppHandle, editing: State<Editing>, every_ms: u64) -> Result<(), String> {
    finish(&app, &editing, "tool:parts", |view| match view {
        View::Parts { length_ms, .. } if convert::parts_count(*length_ms, every_ms).is_some() => Ok(Edit::Parts(every_ms)),
        View::Parts { .. } => Err("that makes fewer than two parts, or too many".into()),
        _ => Err("this window doesn't split".into()),
    })
}

/// Adjust: the sliders, within what the window offers, with something changed.
#[tauri::command]
pub fn editor_adjust(app: AppHandle, editing: State<Editing>, adjust: convert::Adjust) -> Result<(), String> {
    finish(&app, &editing, "tool:adjust", |view| match view {
        View::Adjust { .. } => adjust.check().map(|()| Edit::Adjust(adjust)),
        _ => Err("this window doesn't adjust".into()),
    })
}

/// Mark Up: the shapes, every one inside the picture and from the window's tools.
#[tauri::command]
pub fn editor_markup(app: AppHandle, editing: State<Editing>, markup: convert::markup::Markup) -> Result<(), String> {
    finish(&app, &editing, "tool:markup", |view| match view {
        View::Markup { width, height, video: false, .. } => markup.check(*width, *height).map(|()| Edit::Markup(markup)),
        _ => Err("this window doesn't mark up".into()),
    })
}

/// Black Out: black boxes only, each inside the video's frame, then the job is queued.
#[tauri::command]
pub fn editor_blackout(app: AppHandle, editing: State<Editing>, markup: convert::markup::Markup) -> Result<(), String> {
    finish(&app, &editing, "tool:blackout", |view| match view {
        View::Markup { width, height, video: true, .. } => {
            markup.video_boxes(*width, *height).map(|_| Edit::Markup(markup))
        }
        _ => Err("this window doesn't black out a video".into()),
    })
}

/// Reorder: every page once, in an order that isn't the old one.
#[tauri::command]
pub fn editor_order(app: AppHandle, editing: State<Editing>, order: Vec<u32>) -> Result<(), String> {
    finish(&app, &editing, "tool:order", |view| match view {
        View::Order { pages } => convert::check_order(&order, pages.len()).map(|()| Edit::Order(order)),
        _ => Err("this window doesn't reorder".into()),
    })
}

/// Info: changed tags, only ones the window offered for this file.
#[tauri::command]
pub fn editor_info(app: AppHandle, editing: State<Editing>, changes: Vec<(String, String)>) -> Result<(), String> {
    finish(&app, &editing, "tool:info", |view| match view {
        View::Info { fields, .. } => convert::info::check(&changes, fields).map(|()| Edit::Info(changes)),
        _ => Err("this window doesn't change tags".into()),
    })
}

/// Snapshot's window: the frame at `ms`, as a JPEG data: URL. On a thread of its own,
/// as ffmpeg takes a moment.
#[tauri::command(async)]
pub fn editor_frame(editing: State<'_, Editing>, ms: u64) -> Result<String, String> {
    let path = match editing.lock().as_ref() {
        Some(Session { path, view: View::Snapshot { length_ms }, .. }) if ms <= *length_ms => path.clone(),
        Some(Session { view: View::Snapshot { .. }, .. }) => return Err("that moment is past the end".into()),
        _ => return Err("this window shows no video".into()),
    };
    convert::frame_preview(&path, ms).map(|jpeg| data_url("image/jpeg", &jpeg))
}

/// Snapshot: a moment within the video.
#[tauri::command]
pub fn editor_snapshot(app: AppHandle, editing: State<Editing>, ms: u64) -> Result<(), String> {
    finish(&app, &editing, "tool:snapshot", |view| match view {
        View::Snapshot { length_ms } if ms <= *length_ms => Ok(Edit::Snapshot(ms)),
        View::Snapshot { .. } => Err("that moment is past the end".into()),
        _ => Err("this window takes no snapshot".into()),
    })
}

#[tauri::command]
pub fn editor_cancel(app: AppHandle, editing: State<Editing>) {
    *editing.lock() = None;
    close_window(&app);
}
