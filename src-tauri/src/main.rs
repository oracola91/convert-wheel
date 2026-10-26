#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod archive;
mod convert;
mod converters;
#[cfg(target_os = "macos")]
mod drop_view;
mod formats;
#[cfg(target_os = "linux")]
mod linux;
mod log;
mod login;
#[cfg(target_os = "macos")]
mod mac_notification;
mod menu;
mod reveal;
mod settings;
mod trigger;
mod update;
mod wheel;
#[cfg(target_os = "macos")]
mod wheel_window;
#[cfg(windows)]
mod win32;

use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, RunEvent};
use tauri_plugin_notification::NotificationExt;

/// Drops queued or being converted. Quitting and updating wait for, or warn about, these.
static JOBS: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn busy_jobs() -> usize {
    JOBS.load(Ordering::SeqCst)
}

struct Job {
    paths: Vec<String>,
    format: String,
}

/// How the menu and the notifications speak of a drop: converting it to a format, or
/// a tool from the Tools wheel. `{}` stands for the file's name, or "3 of 10".
struct Words {
    /// "Converting {} to PNG": the progress line.
    doing: String,
    /// "Converted to PNG": the title over the new file's name.
    done_one: String,
    /// "Converted {} files to PNG", with the count.
    done_many: String,
    /// "convert", as in "Couldn't convert photo.heic".
    verb: &'static str,
    /// "Stopped converting to PNG"
    stopped: String,
}

impl Words {
    fn new(target: &str) -> Words {
        let tool = |doing: &str, done_one: &str, done_many: &str, verb, stopped: &str| Words {
            doing: doing.to_string(),
            done_one: done_one.to_string(),
            done_many: done_many.to_string(),
            verb,
            stopped: stopped.to_string(),
        };
        match target {
            "tool:compress" => tool("Compressing {}", "Compressed", "Compressed {} files", "compress", "Stopped compressing"),
            "tool:half" => tool(
                "Resizing {} to half size",
                "Resized to half size",
                "Resized {} files to half size",
                "resize",
                "Stopped resizing",
            ),
            "tool:clean" => tool(
                "Removing metadata from {}",
                "Removed metadata",
                "Removed metadata from {} files",
                "remove metadata from",
                "Stopped removing metadata",
            ),
            "tool:rotate" => tool("Rotating {}", "Rotated", "Rotated {} files", "rotate", "Stopped rotating"),
            // Merge makes one file of the whole drop: "{}" is "3 files".
            "tool:merge" => tool("Merging {}", "Merged into one PDF", "Merged {} files", "merge", "Stopped merging"),
            "tool:onepdf" => tool(
                "Making one PDF of {}",
                "Made one PDF",
                "Made one PDF of {} files",
                "make one PDF of",
                "Stopped making the PDF",
            ),
            "tool:split" => tool(
                "Splitting {} into pages",
                "Split into pages",
                "Split {} files into pages",
                "split",
                "Stopped splitting",
            ),
            "tool:smaller" => tool("Shrinking {}", "Made smaller", "Made {} files smaller", "shrink", "Stopped shrinking"),
            "tool:still" => tool(
                "Taking the first frame of {}",
                "Saved the first frame",
                "Saved the first frame of {} files",
                "take the first frame of",
                "Stopped taking frames",
            ),
            "tool:loudness" => tool(
                "Evening out the loudness of {}",
                "Evened out the loudness",
                "Evened out the loudness of {} files",
                "even out the loudness of",
                "Stopped evening out the loudness",
            ),
            "tool:mono" => tool("Mixing {} down to mono", "Made mono", "Made {} files mono", "mix down", "Stopped mixing down"),
            "act:zip" => tool("Making a ZIP of {}", "Made a ZIP", "Made a ZIP of {} files", "zip", "Stopped making the ZIP"),
            "act:tgz" => tool("Making a TAR.GZ of {}", "Made a TAR.GZ", "Made a TAR.GZ of {} files", "pack", "Stopped packing"),
            "act:tar" => tool("Making a TAR of {}", "Made a TAR", "Made a TAR of {} files", "pack", "Stopped packing"),
            "act:unpack" => tool("Unpacking {}", "Unpacked", "Unpacked {} archives", "unpack", "Stopped unpacking"),
            format => {
                let label = format.to_uppercase();
                Words {
                    doing: format!("Converting {{}} to {label}"),
                    done_one: format!("Converted to {label}"),
                    done_many: format!("Converted {{}} files to {label}"),
                    verb: "convert",
                    stopped: format!("Stopped converting to {label}"),
                }
            }
        }
    }
}

/// The drop being converted: how far it has got, for the menu, and its Stop switch.
struct Running {
    control: Arc<convert::Control>,
    target: String,
    words: Words,
    count: usize,
    /// The file being converted: its place in the drop (from 1), its name, and when
    /// it started.
    file: Mutex<(usize, String, Instant)>,
}

impl Running {
    fn new(target: &str, paths: &[String]) -> Self {
        // A tool that makes one file of the whole drop works on it as one thing.
        let (count, first) = if formats::combines(target) {
            let what = match paths {
                [one] => file_name(Path::new(one)),
                many => format!("{} files", many.len()),
            };
            (1, what)
        } else {
            (paths.len(), paths.first().map(|p| file_name(Path::new(p))).unwrap_or_default())
        };
        Running {
            control: convert::Control::new(),
            target: target.to_string(),
            words: Words::new(target),
            count,
            file: Mutex::new((1, first, Instant::now())),
        }
    }

    fn at(&self, place: usize, name: String) {
        *self.file.lock().unwrap_or_else(|e| e.into_inner()) = (place, name, Instant::now());
    }

    fn line(&self) -> String {
        let (place, name, since) = self.file.lock().unwrap_or_else(|e| e.into_inner()).clone();
        progress_line(&self.words, self.count, place, &name, self.control.percent(), since.elapsed())
    }
}

/// Longer file names are cut in the progress line, so the percent after them still
/// fits in the menu.
const NAME_MAX_CHARS: usize = 32;

/// The menu's progress line: "Converting holiday.mov to MP4… 42%", or for a batch
/// "Converting 3 of 10 to PNG…". A tool that doesn't say how far it is gets the time
/// so far instead, once that is long enough to wonder whether it is stuck.
fn progress_line(words: &Words, count: usize, place: usize, name: &str, percent: Option<u8>, elapsed: Duration) -> String {
    let what = if count == 1 {
        let name = if name.chars().count() > NAME_MAX_CHARS {
            name.chars().take(NAME_MAX_CHARS - 1).collect::<String>() + "…"
        } else {
            name.to_string()
        };
        words.doing.replace("{}", &name) + "…"
    } else {
        words.doing.replace("{}", &format!("{place} of {count}")) + "…"
    };
    match (percent, elapsed.as_secs()) {
        (Some(p), _) => format!("{what} {p}%"),
        (None, s) if s < 10 => what,
        (None, s) if s < 60 => format!("{what} {s} s"),
        (None, s) => format!("{what} {} min", s / 60),
    }
}

/// The drop being converted now, if any, for Stop Converting in the menu.
#[derive(Default)]
struct NowConverting(Mutex<Option<Arc<Running>>>);

impl NowConverting {
    fn set(&self, running: Option<Arc<Running>>) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = running;
    }
}

/// Stop Converting: ends the drop being converted now. Drops queued after it still run.
pub(crate) fn stop_converting(app: &AppHandle) {
    if let Some(running) = app.state::<NowConverting>().0.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        log::line(&format!("stop asked while running {}", running.target));
        running.control.stop();
    }
}

/// Esc or a click in the middle of a wheel that was opened from the keyboard shortcut.
#[tauri::command]
fn wheel_cancel(app: AppHandle) {
    trigger::hide_wheel(&app, false, None);
}

/// Queues a drop on the wheel for conversion.
pub(crate) fn enqueue(app: &AppHandle, paths: Vec<String>, format: String) {
    JOBS.fetch_add(1, Ordering::SeqCst);
    if app.state::<Sender<Job>>().send(Job { paths, format }).is_err() {
        JOBS.fetch_sub(1, Ordering::SeqCst);
        log::line("conversion worker is gone; drop not converted");
        notify(app, "Convert Wheel stopped converting", "Quit Convert Wheel and open it again.");
    }
}

/// One worker converts the queued drops in order, so LibreOffice never runs twice at once.
fn start_worker(app: AppHandle) -> Sender<Job> {
    let (tx, rx) = mpsc::channel::<Job>();
    thread::spawn(move || {
        for job in rx {
            let running = Arc::new(Running::new(&job.format, &job.paths));
            app.state::<NowConverting>().set(Some(Arc::clone(&running)));
            menu::show_progress(&app, &running.line());
            let (finished, done_signal) = mpsc::channel::<()>();
            let (title, body, done) = thread::scope(|s| {
                let (app, running) = (&app, &running);
                s.spawn(move || keep_progress_shown(app, running, done_signal));
                // A bug in one job must not end the worker, or every later drop would
                // vanish without a word. (The panic itself is logged by the panic hook.)
                let result = panic::catch_unwind(AssertUnwindSafe(|| {
                    convert::under(&running.control, || run_job(app, &job, running))
                }))
                .unwrap_or_else(|_| {
                    let body = "Convert Wheel hit an internal error.".to_string();
                    (format!("Couldn't {} the files", running.words.verb), body, Vec::new())
                });
                drop(finished);
                result
            });
            app.state::<NowConverting>().set(None);
            JOBS.fetch_sub(1, Ordering::SeqCst);
            let something = !done.is_empty();
            notify_done(&app, &title, &body, done);
            // The menu keeps the reason too, for when notifications are off, and a
            // click on it shows the result like a click on the notification.
            menu::show_last_result(&app, &format!("{title}: {body}"), something);
        }
    });
    tx
}

/// Keeps the menu's progress line current, twice a second, until `finished` closes.
fn keep_progress_shown(app: &AppHandle, running: &Running, finished: Receiver<()>) {
    let mut shown = String::new();
    while let Err(RecvTimeoutError::Timeout) = finished.recv_timeout(Duration::from_millis(500)) {
        let line = running.line();
        if line != shown {
            menu::show_progress(app, &line);
            shown = line;
        }
    }
}

/// Converts a drop's files. Returns the notification's title and text, and the files
/// it produced.
fn run_job(app: &AppHandle, job: &Job, running: &Running) -> (String, String, Vec<PathBuf>) {
    let words = &running.words;
    // Converting or compressing video is slow; the video tools that copy are quick.
    let encodes = !formats::is_tool(&job.format) || job.format == "tool:compress";
    let slow = encodes
        && job.paths.iter().any(|p| {
            formats::extension(Path::new(p)).and_then(|e| formats::kind_of(&e)) == Some(formats::Kind::Video)
        });
    if slow {
        notify(
            app,
            &(words.doing.replace("{} ", "").replace(" {}", "") + "…"),
            "Video can take a while. The menu shows how far it is, and can stop it.",
        );
    }
    // The system's Downloads folder, which also finds one that was moved or renamed.
    let downloads = app.path().download_dir().unwrap_or_else(|_| convert::downloads());
    let save_in = match app.state::<settings::Store>().get().save_to {
        settings::SaveTo::NextToOriginal => None,
        settings::SaveTo::Downloads => Some(downloads.clone()),
    };
    let mut done: Vec<PathBuf> = Vec::new();
    let mut failed: Vec<(String, String)> = Vec::new();
    // Results that ended up in Downloads although they were meant to go next to the original.
    let mut moved = false;
    let combines = formats::combines(&job.format);
    if combines {
        let srcs: Vec<PathBuf> = job.paths.iter().map(PathBuf::from).collect();
        let what = match srcs.as_slice() {
            [one] => file_name(one),
            many => format!("{} files", many.len()),
        };
        match combine(&srcs, &job.format, save_in.as_deref(), &downloads) {
            Ok(out) => {
                log::line(&format!("made {} of {what}", file_name(&out)));
                moved = save_in.is_none() && out.parent() != srcs[0].parent();
                done.push(out);
            }
            Err(_) if running.control.stopped() => {}
            Err(e) => {
                log::line(&format!("failed to combine {what}: {e}"));
                failed.push((what, converters::offer(app, e)));
            }
        }
    }
    for (place, p) in job.paths.iter().enumerate().filter(|_| !combines) {
        if running.control.stopped() {
            break;
        }
        let src = Path::new(p);
        running.at(place + 1, file_name(src));
        match convert_one(src, &job.format, save_in.as_deref(), &downloads) {
            Ok(out) => {
                log::line(&format!("converted {} to {}", file_name(src), file_name(&out)));
                moved |= save_in.is_none() && out.parent() != src.parent();
                done.push(out);
            }
            Err(_) if running.control.stopped() => break,
            Err(e) => {
                log::line(&format!("failed {}: {e}", file_name(src)));
                failed.push((file_name(src), converters::offer(app, e)));
            }
        }
    }
    // Stopped before the end: a Stop that comes once the last file is done changes nothing.
    let expected = if combines { 1 } else { job.paths.len() };
    if done.len() + failed.len() < expected {
        log::line(&format!("stopped after {} of {} files", done.len() + failed.len(), job.paths.len()));
        let body = match done.len() {
            0 => "Nothing was converted.".to_string(),
            n => format!("{n} of {} files done.", job.paths.len()),
        };
        return (words.stopped.clone(), body, done);
    }
    let (title, mut body) = summary(words, &done, &failed);
    if moved {
        body.push_str(" (saved in Downloads: the original's folder is read-only)");
    }
    (title, body, done)
}

/// One file of a drop: converted, run through a tool, or unpacked.
fn convert_one(src: &Path, target: &str, save_in: Option<&Path>, downloads: &Path) -> Result<PathBuf, String> {
    if target == "act:unpack" {
        archive::unpack(src, save_in, downloads)
    } else {
        convert::convert(src, target, save_in, downloads)
    }
}

/// A whole drop made into one file: merged, put into one PDF, or packed.
fn combine(srcs: &[PathBuf], target: &str, save_in: Option<&Path>, downloads: &Path) -> Result<PathBuf, String> {
    if target.starts_with("act:") {
        archive::make(srcs, target, save_in, downloads)
    } else {
        convert::combine(srcs, target, save_in, downloads)
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn summary(words: &Words, done: &[PathBuf], failed: &[(String, String)]) -> (String, String) {
    let names = |list: Vec<String>| {
        let mut s = list.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
        if list.len() > 3 {
            s.push_str(&format!(" and {} more", list.len() - 3));
        }
        s
    };
    match (done.len(), failed.len()) {
        (_, 0) if done.len() == 1 => (words.done_one.clone(), file_name(&done[0])),
        (n, 0) => (
            words.done_many.replace("{}", &n.to_string()),
            names(done.iter().map(|p| file_name(p)).collect()),
        ),
        (0, 1) => (format!("Couldn't {} {}", words.verb, failed[0].0), failed[0].1.clone()),
        (ok, bad) => (
            format!("{bad} of {} files failed", ok + bad),
            format!("{}: {}", failed[0].0, failed[0].1),
        ),
    }
}

/// Numbers every notification, so a click on one can be matched to the drop it reported.
static NOTIFICATION_ID: AtomicI32 = AtomicI32::new(1);

/// Shows a notification. If the system refuses, the message goes to the log, and the
/// menu's last-result line still shows how the last drop went.
pub(crate) fn notify(app: &AppHandle, title: &str, body: &str) {
    show_notification(app, NOTIFICATION_ID.fetch_add(1, Ordering::SeqCst), title, body, &[]);
}

/// `notify` for a finished drop. A click on the notification shows `done`, the files
/// it produced, in the file manager.
fn notify_done(app: &AppHandle, title: &str, body: &str, done: Vec<PathBuf>) {
    let id = NOTIFICATION_ID.fetch_add(1, Ordering::SeqCst);
    show_notification(app, id, title, body, &done);
    app.state::<reveal::Results>().finished(id, done);
}

/// On macOS a click on the notification shows `files`; elsewhere the click is matched
/// to the drop by `id`.
fn show_notification(app: &AppHandle, id: i32, title: &str, body: &str, files: &[PathBuf]) {
    #[cfg(target_os = "macos")]
    {
        let _ = id;
        mac_notification::show(app, title, body, files);
    }
    #[cfg(not(target_os = "macos"))]
    if let Err(e) = app.notification().builder().id(id).title(title).body(body).show() {
        log::line(&format!("notification not shown ({e}): {title}: {body}"));
    }
}

/// `convert-wheel --convert <format> <file>...` converts without the wheel, for scripts and tests.
fn cli(args: &[String]) -> i32 {
    let [format, files @ ..] = args else {
        eprintln!("usage: convert-wheel --convert <format> <file>...");
        return 2;
    };
    if files.is_empty() {
        eprintln!("usage: convert-wheel --convert <format> <file>...");
        return 2;
    }
    let downloads = convert::downloads();
    if formats::combines(format) {
        let srcs: Result<Vec<PathBuf>, String> = files.iter().map(|f| std::path::absolute(f).map_err(|e| e.to_string())).collect();
        return match srcs.and_then(|srcs| combine(&srcs, format, None, &downloads)) {
            Ok(out) => {
                println!("{}", out.display());
                0
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        };
    }
    let mut code = 0;
    for f in files {
        // A full path, so a file name can't be taken for one of a tool's options.
        let result = std::path::absolute(f)
            .map_err(|e| e.to_string())
            .and_then(|src| convert_one(&src, format, None, &downloads));
        match result {
            Ok(out) => println!("{}", out.display()),
            Err(e) => {
                eprintln!("{f}: {e}");
                code = 1;
            }
        }
    }
    code
}

fn main() {
    // Panics anywhere (trigger threads, the worker, update tasks) leave a trace.
    panic::set_hook(Box::new(|info| {
        let place = info.location().map(|l| format!(" at {}:{}", l.file(), l.line())).unwrap_or_default();
        let what = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        log::line(&format!("internal error{place}: {what}"));
    }));

    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--convert") {
        // A Windows release build has no console of its own; borrow the one it was
        // started from so the results and errors show up there.
        #[cfg(windows)]
        unsafe {
            let _ = windows::Win32::System::Console::AttachConsole(windows::Win32::System::Console::ATTACH_PARENT_PROCESS);
        }
        std::process::exit(cli(&args[1..]));
    }
    // For a keyboard shortcut on Linux, where Wayland hides Shift+drag from apps.
    #[cfg(target_os = "linux")]
    if args.first().map(String::as_str) == Some("--show") {
        if let Err(e) = linux::send_show() {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }

    let app = tauri::Builder::default()
        // First, so a second copy hands over to the running one and quits before it
        // opens a second wheel and menu icon. `--update` from a second copy only checks:
        // installing is for the menu or a fresh start with `--update`.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if args.iter().any(|a| a == "--update") {
                update::check_soon(app);
            } else {
                notify(app, "Convert Wheel is already running", "Hold Shift while dragging a file to open the wheel.");
            }
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            // Anything that stops the start is logged and shown; otherwise the app
            // would vanish without a word.
            setup(app).map_err(|e| {
                log::line(&format!("couldn't start: {e}"));
                notify(app.handle(), "Convert Wheel couldn't start", &e.to_string());
                e
            })
        })
        .invoke_handler(tauri::generate_handler![wheel_cancel])
        .build(tauri::generate_context!());
    match app {
        Ok(app) => app.run(|_, event| {
            // Tools still running would otherwise carry on after the app is gone.
            if let RunEvent::Exit = event {
                convert::stop_all();
            }
        }),
        Err(e) => {
            log::line(&format!("couldn't start: {e}"));
            std::process::exit(1);
        }
    }
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    // Menu bar style app: no Dock icon, no app switcher entry.
    #[cfg(target_os = "macos")]
    app.set_activation_policy(tauri::ActivationPolicy::Accessory);

    log::init(app.path().app_log_dir().ok());
    log::line(&format!("started, version {}", app.package_info().version));
    let (store, settings_problem) = settings::Store::load(app.path().app_config_dir()?.join("settings.json"));
    app.manage(store);
    app.manage(update::Pending::default());
    app.manage(reveal::Results::default());
    app.manage(NowConverting::default());
    app.manage(converters::Missing::default());
    // Before any notification: the ones shown earlier never report a click.
    #[cfg(target_os = "macos")]
    mac_notification::install(app.handle());
    let handle = app.handle().clone();
    app.notification().on_action(move |action| {
        if action.action_id() == "tap" {
            if let Some(shown) = action.notification() {
                reveal::show_notified(&handle, shown.id());
            }
        }
    })?;
    menu::build(app)?;
    app.manage(start_worker(app.handle().clone()));
    app.manage(wheel::WheelState::default());
    #[cfg(target_os = "macos")]
    {
        let mtm = objc2::MainThreadMarker::new().ok_or("setup isn't on the main thread")?;
        wheel_window::install(app.handle(), mtm)?;
        drop_view::install(app.handle())?;
    }
    #[cfg(windows)]
    win32::install(app.handle());
    #[cfg(target_os = "linux")]
    linux::install(app.handle());
    trigger::start(app.handle().clone());
    update::start(app.handle().clone());
    if let Some(problem) = settings_problem {
        notify(app.handle(), "Convert Wheel settings", &problem);
    }
    if std::env::args().any(|a| a == "--update") {
        update::install_soon(app.handle());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_progress_line_says_how_far_a_drop_is() {
        let secs = Duration::from_secs;
        assert_eq!(progress_line(&Words::new("mp4"), 1, 1, "holiday.mov", Some(42), secs(90)), "Converting holiday.mov to MP4… 42%");
        assert_eq!(progress_line(&Words::new("png"), 10, 3, "c.heic", None, secs(1)), "Converting 3 of 10 to PNG…");
        assert_eq!(progress_line(&Words::new("png"), 10, 3, "c.heic", Some(7), secs(1)), "Converting 3 of 10 to PNG… 7%");
        // Without a percent, the time so far, once it is worth showing.
        assert_eq!(progress_line(&Words::new("pdf"), 1, 1, "a.docx", None, secs(9)), "Converting a.docx to PDF…");
        assert_eq!(progress_line(&Words::new("pdf"), 1, 1, "a.docx", None, secs(45)), "Converting a.docx to PDF… 45 s");
        assert_eq!(progress_line(&Words::new("pdf"), 1, 1, "a.docx", None, secs(150)), "Converting a.docx to PDF… 2 min");
        // A long name is cut, so the percent stays within the menu's line.
        let long = "Family holiday in the mountains, summer 2026, part one.mov";
        let line = progress_line(&Words::new("webm"), 1, 1, long, Some(42), secs(90));
        assert_eq!(line, "Converting Family holiday in the mountains… to WEBM… 42%");
        assert!(format!("Last: {line}").chars().count() <= menu::LAST_MAX_CHARS);
    }

    #[test]
    fn tools_have_words_of_their_own() {
        let secs = Duration::from_secs;
        let half = Words::new("tool:half");
        assert_eq!(progress_line(&half, 1, 1, "photo.jpg", None, secs(1)), "Resizing photo.jpg to half size…");
        let clean = Words::new("tool:clean");
        assert_eq!(progress_line(&clean, 10, 3, "c.heic", None, secs(1)), "Removing metadata from 3 of 10…");
        let one = [PathBuf::from("/tmp/photo (no metadata).jpg")];
        assert_eq!(summary(&clean, &one, &[]), ("Removed metadata".into(), "photo (no metadata).jpg".into()));
        let two = [PathBuf::from("/tmp/a (compressed).jpg"), PathBuf::from("/tmp/b (compressed).jpg")];
        assert_eq!(summary(&Words::new("tool:compress"), &two, &[]).0, "Compressed 2 files");
        let merge = Words::new("tool:merge");
        let running = Running::new("tool:merge", &["/tmp/a.pdf".to_string(), "/tmp/b.pdf".to_string()]);
        assert_eq!(running.line(), "Merging 2 files…");
        assert_eq!(summary(&merge, &[PathBuf::from("/tmp/a (merged).pdf")], &[]).0, "Merged into one PDF");
        let failed = [("a.png".to_string(), "it is already as small as Compress makes it".to_string())];
        assert_eq!(summary(&Words::new("tool:compress"), &[], &failed).0, "Couldn't compress a.png");
        assert_eq!(Running::new("tool:onepdf", &["/a.jpg".into(), "/b.jpg".into()]).line(), "Making one PDF of 2 files…");
        // Formats keep their words.
        assert_eq!(summary(&Words::new("png"), &[PathBuf::from("/tmp/a.png")], &[]).0, "Converted to PNG");
        assert_eq!(Words::new("mp4").doing.replace("{} ", "") + "…", "Converting to MP4…");
    }

    #[test]
    fn the_first_line_already_names_the_file() {
        let running = Running::new("png", &["/tmp/photo.heic".to_string()]);
        assert_eq!(running.line(), "Converting photo.heic to PNG…");
    }
}
