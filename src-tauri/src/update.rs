//! Updates from the project's GitHub releases.
//!
//! Every update file is signed with the project's update key, and the signature also
//! records the version it was made for. The updater checks both against the public key
//! built into the app before it installs anything, so a changed, fake or relabelled
//! older file is refused. The app looks shortly after it starts and then once a day;
//! when a new version is there, a notification says so and the menu offers to install
//! it. Nothing installs without the user choosing it (or starting the app with
//! `--update`).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Updater, UpdaterExt};

use crate::wheel::lock;
use crate::{log, menu, notify};

const FIRST_CHECK: Duration = Duration::from_secs(30);
const EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// A stalled connection must not hang a check or a download for ever.
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// After this many failed daily checks in a row, say so once instead of only logging.
const QUIET_FAILURES: u32 = 3;
const RELEASES: &str = "https://github.com/oracola91/convert-wheel/releases";

/// The newer version found by the last check, if any.
#[derive(Default)]
pub struct Pending(Mutex<Option<String>>);

/// A check or an install is under way; a second one waits its turn.
static BUSY: AtomicBool = AtomicBool::new(false);
static FAILED_IN_A_ROW: AtomicU32 = AtomicU32::new(0);

pub fn pending(app: &AppHandle) -> Option<String> {
    lock(&app.state::<Pending>().0).clone()
}

pub fn start(app: AppHandle) {
    thread::spawn(move || {
        thread::sleep(FIRST_CHECK);
        loop {
            tauri::async_runtime::block_on(check(&app, false));
            thread::sleep(EVERY);
        }
    });
}

/// "Check for Updates" in the menu.
pub fn check_soon(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move { check(&app, true).await });
}

/// "Install Update" in the menu, or starting the app with `--update`. Conversions
/// still running or queued would be cut off by the restart, so it waits for them.
pub fn install_soon(app: &AppHandle) {
    let app = app.clone();
    thread::spawn(move || {
        if crate::busy_jobs() > 0 {
            notify(&app, "Updating after the current conversions", "Convert Wheel installs the update when they're done.");
            while crate::busy_jobs() > 0 {
                thread::sleep(Duration::from_secs(2));
            }
        }
        tauri::async_runtime::block_on(install(&app));
    });
}

/// Marks an update action as running; false if one already is.
fn claim(app: &AppHandle, asked: bool) -> bool {
    if BUSY.swap(true, Ordering::SeqCst) {
        if asked {
            notify(app, "Convert Wheel is already checking for updates", "It will say when it's done.");
        }
        return false;
    }
    true
}

fn release() {
    BUSY.store(false, Ordering::SeqCst);
}

fn updater(app: &AppHandle, timeout: Duration) -> tauri_plugin_updater::Result<Updater> {
    app.updater_builder()
        .timeout(timeout)
        // Windows: the installer takes over from here; leave a trace before the app exits.
        .on_before_exit(|| log::line("handing over to the update installer"))
        .build()
}

/// Looks for a newer version. `asked`: the user chose Check for Updates, so also say
/// when there is nothing new or the check failed.
async fn check(app: &AppHandle, asked: bool) {
    if !claim(app, asked) {
        return;
    }
    let found = match updater(app, CHECK_TIMEOUT) {
        Ok(updater) => updater.check().await,
        Err(e) => Err(e),
    };
    release();
    match found {
        Ok(Some(update)) => {
            FAILED_IN_A_ROW.store(0, Ordering::SeqCst);
            let version = update.version.clone();
            let is_new = lock(&app.state::<Pending>().0).replace(version.clone()).as_deref() != Some(version.as_str());
            menu::show_update(app, menu::UpdateState::Available(&version));
            if asked || is_new {
                log::line(&format!("update {version} available"));
                notify(
                    app,
                    &format!("Convert Wheel {version} is available"),
                    "Choose Install Update in the Convert Wheel menu.",
                );
            }
        }
        Ok(None) => {
            FAILED_IN_A_ROW.store(0, Ordering::SeqCst);
            *lock(&app.state::<Pending>().0) = None;
            menu::show_update(app, menu::UpdateState::UpToDate);
            if asked {
                let current = app.package_info().version.to_string();
                notify(app, "Convert Wheel is up to date", &format!("You have version {current}."));
            }
        }
        Err(e) => {
            log::line(&format!("update check failed: {e}"));
            menu::show_update(app, menu::UpdateState::CheckFailed);
            let failures = FAILED_IN_A_ROW.fetch_add(1, Ordering::SeqCst) + 1;
            if asked || failures == QUIET_FAILURES {
                notify(app, "Couldn't check for updates", &format!("{e}. New versions are also at {RELEASES}"));
            }
        }
    }
}

/// Downloads, verifies and installs the newest version, then starts it.
async fn install(app: &AppHandle) {
    if !claim(app, true) {
        return;
    }
    menu::show_update(app, menu::UpdateState::Installing);
    let found = match updater(app, DOWNLOAD_TIMEOUT) {
        Ok(updater) => updater.check().await,
        Err(e) => Err(e),
    };
    let update = match found {
        Ok(Some(update)) => update,
        Ok(None) => {
            release();
            *lock(&app.state::<Pending>().0) = None;
            menu::show_update(app, menu::UpdateState::UpToDate);
            notify(app, "Convert Wheel is up to date", "There is no newer version.");
            return;
        }
        Err(e) => {
            release();
            log::line(&format!("update check failed: {e}"));
            menu::show_update(app, menu::UpdateState::CheckFailed);
            notify(app, "Couldn't check for updates", &format!("{e}. New versions are also at {RELEASES}"));
            return;
        }
    };
    let version = update.version.clone();
    notify(app, &format!("Installing Convert Wheel {version}…"), "It restarts by itself when it's done.");
    match update.download_and_install(|_, _| {}, || {}).await {
        Ok(()) => {
            log::line(&format!("installed update {version}, restarting"));
            relaunch(app);
        }
        Err(e) => {
            release();
            log::line(&format!("update {version} failed: {e}"));
            menu::show_update(app, menu::UpdateState::Available(&version));
            notify(
                app,
                "Couldn't install the update",
                &format!("{e}. You can download version {version} from {RELEASES}"),
            );
        }
    }
}

/// Starts the updated app once this one has quit. A plain restart could start the new
/// copy while this one still holds the "already running" lock, and the new copy would
/// then quit too. Windows never gets here: its installer restarts the app.
fn relaunch(app: &AppHandle) {
    #[cfg(unix)]
    {
        let pid = std::process::id().to_string();
        let target = relaunch_target();
        let script = if cfg!(target_os = "macos") {
            r#"while kill -0 "$1" 2>/dev/null; do sleep 0.2; done; open "$2""#
        } else {
            r#"while kill -0 "$1" 2>/dev/null; do sleep 0.2; done; exec "$2""#
        };
        if let Some(target) = target {
            let started = std::process::Command::new("/bin/sh")
                .args(["-c", script, "sh", &pid])
                .arg(target)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
            if let Err(e) = started {
                log::line(&format!("couldn't schedule the restart ({e}); start Convert Wheel again by hand"));
            }
        }
        app.exit(0);
    }
    #[cfg(not(unix))]
    app.restart();
}

/// What to start after an update: the .app bundle on macOS, the AppImage file or the
/// installed program on Linux.
#[cfg(unix)]
fn relaunch_target() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    if cfg!(target_os = "macos") {
        // .../Convert Wheel.app/Contents/MacOS/convert-wheel
        return exe.ancestors().nth(3).map(|p| p.to_path_buf());
    }
    std::env::var_os("APPIMAGE").map(std::path::PathBuf::from).or(Some(exe))
}
