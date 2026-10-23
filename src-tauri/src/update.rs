//! Updates from the project's GitHub releases.
//!
//! Every update file is signed with the project's update key, and the updater checks
//! it against the public key built into the app before it installs anything, so a
//! changed or fake file is refused. The app looks shortly after it starts and then
//! once a day; when a new version is there, a notification says so and the menu
//! offers to install it. Nothing installs without the user choosing it (or running
//! `convert-wheel --update`).

use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Manager};
use tauri_plugin_updater::UpdaterExt;

use crate::wheel::lock;
use crate::{log, menu, notify};

const FIRST_CHECK: Duration = Duration::from_secs(30);
const EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// The newer version found by the last check, if any.
#[derive(Default)]
pub struct Pending(Mutex<Option<String>>);

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

/// "Install Update" in the menu, or `convert-wheel --update`.
pub fn install_soon(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move { install(&app).await });
}

/// Looks for a newer version. `asked`: the user chose Check for Updates, so also say
/// when there is nothing new or the check failed.
async fn check(app: &AppHandle, asked: bool) {
    let found = match app.updater() {
        Ok(updater) => updater.check().await,
        Err(e) => Err(e),
    };
    match found {
        Ok(Some(update)) => {
            let version = update.version.clone();
            let is_new = lock(&app.state::<Pending>().0).replace(version.clone()).as_deref() != Some(version.as_str());
            menu::show_update(app, Some(&version));
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
            *lock(&app.state::<Pending>().0) = None;
            menu::show_update(app, None);
            if asked {
                let current = app.package_info().version.to_string();
                notify(app, "Convert Wheel is up to date", &format!("You have version {current}."));
            }
        }
        Err(e) => {
            log::line(&format!("update check failed: {e}"));
            if asked {
                notify(app, "Couldn't check for updates", &e.to_string());
            }
        }
    }
}

/// Downloads, verifies and installs the newest version, then restarts the app.
async fn install(app: &AppHandle) {
    let found = match app.updater() {
        Ok(updater) => updater.check().await,
        Err(e) => Err(e),
    };
    let update = match found {
        Ok(Some(update)) => update,
        Ok(None) => {
            notify(app, "Convert Wheel is up to date", "There is no newer version.");
            return;
        }
        Err(e) => {
            log::line(&format!("update check failed: {e}"));
            notify(app, "Couldn't check for updates", &e.to_string());
            return;
        }
    };
    let version = update.version.clone();
    notify(app, &format!("Installing Convert Wheel {version}…"), "It restarts by itself when it's done.");
    match update.download_and_install(|_, _| {}, || {}).await {
        Ok(()) => {
            log::line(&format!("installed update {version}, restarting"));
            app.restart();
        }
        Err(e) => {
            log::line(&format!("update {version} failed: {e}"));
            notify(app, "Couldn't install the update", &e.to_string());
        }
    }
}
