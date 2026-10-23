#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod convert;
#[cfg(target_os = "macos")]
mod drop_view;
mod formats;
mod trigger;
mod wheel;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

struct Job {
    paths: Vec<String>,
    format: String,
}

/// Queues a drop on the wheel for conversion.
pub(crate) fn enqueue(app: &AppHandle, paths: Vec<String>, format: String) {
    let _ = app.state::<Sender<Job>>().send(Job { paths, format });
}

/// One worker converts the queued drops in order, so LibreOffice never runs twice at once.
fn start_worker(app: AppHandle) -> Sender<Job> {
    let (tx, rx) = mpsc::channel::<Job>();
    thread::spawn(move || {
        for job in rx {
            let label = job.format.to_uppercase();
            let slow = job.paths.iter().any(|p| {
                formats::extension(Path::new(p)).and_then(|e| formats::kind_of(&e)) == Some(formats::Kind::Video)
            });
            if slow {
                notify(&app, &format!("Converting to {label}…"), "Video can take a while.");
            }
            let mut done: Vec<PathBuf> = Vec::new();
            let mut failed: Vec<(String, String)> = Vec::new();
            for p in &job.paths {
                let src = Path::new(p);
                match convert::convert(src, &job.format) {
                    Ok(out) => {
                        eprintln!("convert-wheel: {} -> {}", src.display(), out.display());
                        done.push(out);
                    }
                    Err(e) => {
                        eprintln!("convert-wheel: {} failed: {e}", src.display());
                        failed.push((file_name(src), e));
                    }
                }
            }
            let (title, body) = summary(&label, &done, &failed);
            notify(&app, &title, &body);
        }
    });
    tx
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

fn notify(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
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
    let mut code = 0;
    for f in files {
        match convert::convert(Path::new(f), format) {
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

    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            // Menu bar style app: no Dock icon, no app switcher entry.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let quit = MenuItem::with_id(app, "quit", "Quit Convert Wheel", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&quit])?;
            let mut tray = TrayIconBuilder::new()
                .tooltip("Convert Wheel: hold Shift while dragging a file")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| {
                    if event.id() == "quit" {
                        app.exit(0);
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            app.manage(start_worker(app.handle().clone()));
            app.manage(wheel::Shown::default());
            #[cfg(target_os = "macos")]
            drop_view::install(app.handle())?;
            trigger::start(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running convert-wheel");
}
