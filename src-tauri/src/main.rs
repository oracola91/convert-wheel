#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod convert;
#[cfg(target_os = "macos")]
mod drop_view;
mod formats;
#[cfg(target_os = "linux")]
mod linux;
mod log;
mod login;
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
use std::sync::mpsc::{self, Sender};
use std::thread;

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
            let label = job.format.to_uppercase();
            menu::show_last_result(&app, &format!("Converting to {label}…"), false);
            // A bug in one job must not end the worker, or every later drop would
            // vanish without a word. (The panic itself is logged by the panic hook.)
            let (title, body, done) = panic::catch_unwind(AssertUnwindSafe(|| run_job(&app, &job, &label)))
                .unwrap_or_else(|_| {
                    let body = "Convert Wheel hit an internal error.".to_string();
                    (format!("Couldn't convert to {label}"), body, Vec::new())
                });
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

/// Converts a drop's files. Returns the notification's title and text, and the files
/// it produced.
fn run_job(app: &AppHandle, job: &Job, label: &str) -> (String, String, Vec<PathBuf>) {
    let slow = job.paths.iter().any(|p| {
        formats::extension(Path::new(p)).and_then(|e| formats::kind_of(&e)) == Some(formats::Kind::Video)
    });
    if slow {
        notify(app, &format!("Converting to {label}…"), "Video can take a while.");
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
    for p in &job.paths {
        let src = Path::new(p);
        match convert::convert(src, &job.format, save_in.as_deref(), &downloads) {
            Ok(out) => {
                log::line(&format!("converted {} to {}", file_name(src), file_name(&out)));
                moved |= save_in.is_none() && out.parent() != src.parent();
                done.push(out);
            }
            Err(e) => {
                log::line(&format!("failed {}: {e}", file_name(src)));
                failed.push((file_name(src), e));
            }
        }
    }
    let (title, mut body) = summary(label, &done, &failed);
    if moved {
        body.push_str(" (saved in Downloads: the original's folder is read-only)");
    }
    (title, body, done)
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn summary(label: &str, done: &[PathBuf], failed: &[(String, String)]) -> (String, String) {
    let names = |list: Vec<String>| {
        let mut s = list.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
        if list.len() > 3 {
            s.push_str(&format!(" and {} more", list.len() - 3));
        }
        s
    };
    match (done.len(), failed.len()) {
        (_, 0) if done.len() == 1 => (format!("Converted to {label}"), file_name(&done[0])),
        (n, 0) => (
            format!("Converted {n} files to {label}"),
            names(done.iter().map(|p| file_name(p)).collect()),
        ),
        (0, 1) => (format!("Couldn't convert {}", failed[0].0), failed[0].1.clone()),
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
    show_notification(app, NOTIFICATION_ID.fetch_add(1, Ordering::SeqCst), title, body);
}

/// `notify` for a finished drop. A click on the notification shows `done`, the files
/// it produced, in the file manager.
fn notify_done(app: &AppHandle, title: &str, body: &str, done: Vec<PathBuf>) {
    let id = NOTIFICATION_ID.fetch_add(1, Ordering::SeqCst);
    app.state::<reveal::Results>().finished(id, done);
    show_notification(app, id, title, body);
}

fn show_notification(app: &AppHandle, id: i32, title: &str, body: &str) {
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
    let mut code = 0;
    for f in files {
        // A full path, so a file name can't be taken for one of a tool's options.
        let result = std::path::absolute(f)
            .map_err(|e| e.to_string())
            .and_then(|src| convert::convert(&src, format, None, &downloads));
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
    // Before any notification: the ones shown earlier never report a click.
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
