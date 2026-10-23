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
use std::sync::mpsc::{self, Sender};
use std::thread;

use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

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
    if app.state::<Sender<Job>>().send(Job { paths, format }).is_err() {
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
            // A bug in one job must not end the worker, or every later drop would
            // vanish without a word.
            let (title, body) = panic::catch_unwind(AssertUnwindSafe(|| run_job(&app, &job, &label)))
                .unwrap_or_else(|_| {
                    log::line(&format!("internal error while converting to {label}"));
                    (format!("Couldn't convert to {label}"), "Convert Wheel hit an internal error.".to_string())
                });
            notify(&app, &title, &body);
            menu::show_last_result(&app, &title);
        }
    });
    tx
}

fn run_job(app: &AppHandle, job: &Job, label: &str) -> (String, String) {
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
    (title, body)
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

/// Shows a notification. If the system refuses, the message goes to the log, and the
/// menu's last-result line still shows how the last drop went.
pub(crate) fn notify(app: &AppHandle, title: &str, body: &str) {
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
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
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--convert") {
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

    tauri::Builder::default()
        // First, so a second copy hands over to the running one and quits before it
        // opens a second wheel and menu icon. `--update` asks the running copy to update.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if args.iter().any(|a| a == "--update") {
                update::install_soon(app);
            } else {
                notify(app, "Convert Wheel is already running", "Hold Shift while dragging a file to open the wheel.");
            }
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            // Menu bar style app: no Dock icon, no app switcher entry.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            if let Ok(dir) = app.path().app_log_dir() {
                log::init(dir);
            }
            log::line(&format!("started, version {}", app.package_info().version));
            app.manage(settings::Store::load(app.path().app_config_dir()?.join("settings.json")));
            app.manage(update::Pending::default());
            menu::build(app)?;
            app.manage(start_worker(app.handle().clone()));
            app.manage(wheel::WheelState::default());
            #[cfg(target_os = "macos")]
            {
                let mtm = objc2::MainThreadMarker::new().expect("setup runs on the main thread");
                wheel_window::install(app.handle(), mtm);
                drop_view::install(app.handle())?;
            }
            #[cfg(windows)]
            win32::install(app.handle());
            #[cfg(target_os = "linux")]
            linux::install(app.handle());
            trigger::start(app.handle().clone());
            update::start(app.handle().clone());
            if std::env::args().any(|a| a == "--update") {
                update::install_soon(app.handle());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![wheel_cancel])
        .run(tauri::generate_context!())
        .expect("error while running convert-wheel");
}
