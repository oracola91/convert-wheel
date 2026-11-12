//! Installs the converters that drops found missing, when the menu's "Install …" item
//! is chosen: with winget on Windows, with Homebrew on macOS, and on Linux with the
//! system's own package manager (apt on Debian and Ubuntu, dnf on Fedora, pacman on
//! Arch, zypper on openSUSE) through pkexec, which asks for the admin password in the
//! desktop's own window. Other Linux systems get the message that names the command.
//! "Update Converters…" asks the same package manager for newer versions of the
//! converters it installed. On Windows without winget, App Installer (which brings it)
//! is registered first, and failing that the Microsoft Store opens on it.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};

use crate::convert::{self, Converter};
use crate::{log, menu};

/// Whether this system's converters can be installed from the menu.
pub const AVAILABLE: bool = cfg!(any(target_os = "macos", windows));

/// LibreOffice is a download of several hundred MB, and Homebrew may update itself
/// first; a package manager that hangs must still end.
const LIMIT: Duration = Duration::from_secs(30 * 60);

/// The package manager's name in messages: Homebrew, winget, or the one this Linux
/// system has.
struct PackageManager;

impl std::fmt::Display for PackageManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if cfg!(target_os = "macos") {
            "Homebrew"
        } else if cfg!(windows) {
            "winget"
        } else {
            Linux::here().map_or("A package manager", Linux::name)
        })
    }
}

const PACKAGE_MANAGER: PackageManager = PackageManager;

/// The Linux package managers converters are installed with, through pkexec, in the
/// order they are looked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(any(target_os = "macos", windows), allow(dead_code))]
pub(crate) enum Linux {
    /// Debian, Ubuntu.
    Apt,
    /// Fedora.
    Dnf,
    /// Arch.
    Pacman,
    /// openSUSE.
    Zypper,
}

#[cfg_attr(any(target_os = "macos", windows), allow(dead_code))]
impl Linux {
    const ALL: [Linux; 4] = [Linux::Apt, Linux::Dnf, Linux::Pacman, Linux::Zypper];

    fn program(self) -> &'static str {
        match self {
            Linux::Apt => "/usr/bin/apt-get",
            Linux::Dnf => "/usr/bin/dnf",
            Linux::Pacman => "/usr/bin/pacman",
            Linux::Zypper => "/usr/bin/zypper",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Linux::Apt => "apt",
            Linux::Dnf => "dnf",
            Linux::Pacman => "pacman",
            Linux::Zypper => "zypper",
        }
    }

    /// The one this system has, if any (always None on macOS and Windows).
    pub(crate) fn here() -> Option<Linux> {
        if !cfg!(target_os = "linux") {
            return None;
        }
        Self::ALL.into_iter().find(|pm| Path::new(pm.program()).is_file())
    }

    fn package(self, c: &Converter) -> &'static str {
        match self {
            Linux::Apt => c.apt,
            Linux::Dnf => c.dnf,
            Linux::Pacman => c.pacman,
            Linux::Zypper => c.zypper,
        }
    }

    /// What runs as root, after pkexec, to install `c` without questions.
    fn install(self, c: &Converter) -> Vec<&'static str> {
        let package = self.package(c);
        match self {
            // pkexec clears the environment, so env sets apt's quiet mode for it.
            Linux::Apt => vec!["env", "DEBIAN_FRONTEND=noninteractive", "/usr/bin/apt-get", "install", "-y", package],
            Linux::Dnf => vec!["/usr/bin/dnf", "install", "-y", package],
            // --needed: nothing reinstalled. No -y: Arch updates with the whole system
            // (pacman -Syu), and a list refresh alone would make a partial upgrade.
            Linux::Pacman => vec!["/usr/bin/pacman", "-S", "--needed", "--noconfirm", package],
            Linux::Zypper => vec!["/usr/bin/zypper", "--non-interactive", "install", package],
        }
    }

    /// The command people type for `c`, for the message when it is missing.
    pub(crate) fn hint(self, c: &Converter) -> String {
        let package = self.package(c);
        match self {
            Linux::Apt => format!("sudo apt install {package}"),
            Linux::Dnf => format!("sudo dnf install {package}"),
            Linux::Pacman => format!("sudo pacman -S {package}"),
            Linux::Zypper => format!("sudo zypper install {package}"),
        }
    }
}

/// The converters drops found missing, in the order found: what the Install item offers.
#[derive(Default)]
pub struct Missing(Mutex<Vec<&'static Converter>>);

impl Missing {
    fn list(&self) -> std::sync::MutexGuard<'_, Vec<&'static Converter>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// One install at a time; while it runs, the menu item says what it is installing.
static INSTALLING: AtomicBool = AtomicBool::new(false);

/// A conversion's error, as the notification should put it. When the error says a
/// converter is missing and this system can install it, the menu offers it and the
/// message points there instead of at a command.
pub fn offer(app: &AppHandle, error: String) -> String {
    let Some(c) = convert::missing(&error).filter(|_| available()) else {
        return error;
    };
    {
        let state = app.state::<Missing>();
        let mut missing = state.list();
        if !missing.iter().any(|m| m.tool == c.tool) {
            missing.push(c);
        }
    }
    show(app);
    pointer(c)
}

fn pointer(c: &Converter) -> String {
    format!("{} is not installed. Choose Install {} in the {} menu.", c.name, c.name, crate::APP_NAME)
}

/// Puts the Install item in the menu for the converters still missing, or takes it out.
fn show(app: &AppHandle) {
    if INSTALLING.load(Ordering::SeqCst) {
        return;
    }
    let missing = app.state::<Missing>().list().clone();
    let label = (!missing.is_empty()).then(|| format!("Install {}…", names(&missing)));
    menu::show_install(app, label.as_deref(), true);
}

/// "ffmpeg", "ffmpeg and MuPDF", "ffmpeg, pandoc and MuPDF".
fn names(list: &[&Converter]) -> String {
    match list {
        [] => String::new(),
        [one] => one.name.to_string(),
        [rest @ .., last] => {
            let rest: Vec<&str> = rest.iter().map(|c| c.name).collect();
            format!("{} and {}", rest.join(", "), last.name)
        }
    }
}

/// An install or update is running (Settings keeps its buttons waiting meanwhile).
pub fn busy() -> bool {
    INSTALLING.load(Ordering::SeqCst)
}

/// The converters this system's package manager provides that the app can't find.
pub fn not_installed() -> Vec<&'static Converter> {
    convert::CONVERTERS.iter().filter(|c| updatable(c) && !convert::installed(c)).collect()
}

/// Settings' "Install Missing Converters…" (roadmap stage 6, one-step setup): every
/// converter not found, in one run of the same Install.
pub fn install_all(app: &AppHandle) {
    {
        let state = app.state::<Missing>();
        let mut missing = state.list();
        for c in not_installed() {
            if !missing.iter().any(|m| m.tool == c.tool) {
                missing.push(c);
            }
        }
    }
    install(app);
}

/// The menu's Install item: installs the missing converters one after another, on a
/// thread of its own, and says how it went. It stops at the first that fails; that one
/// and the rest stay in the menu, to try again.
pub fn install(app: &AppHandle) {
    if INSTALLING.swap(true, Ordering::SeqCst) {
        return;
    }
    // Settings shows "Installing…" until it ends, then looks again at what is missing.
    crate::settings_window::converters_installing(app, true);
    let app = app.clone();
    thread::spawn(move || {
        let wanted = app.state::<Missing>().list().clone();
        let mut done: Vec<&Converter> = Vec::new();
        let mut failed: Option<(&Converter, String)> = None;
        let pm = get_package_manager();
        if let Some(pm) = &pm {
            for c in &wanted {
                if !convert::installed(c) {
                    menu::show_install(&app, Some(&format!("Installing {}…", c.name)), false);
                    log::line(&format!("installing {} with {PACKAGE_MANAGER}", c.name));
                    if let Err(e) = install_one(pm, c) {
                        log::line(&format!("couldn't install {}: {e}", c.name));
                        failed = Some((c, e));
                        break;
                    }
                }
                log::line(&format!("{} is installed", c.name));
                done.push(c);
            }
        }
        app.state::<Missing>().list().retain(|m| !done.iter().any(|d| d.tool == m.tool));
        INSTALLING.store(false, Ordering::SeqCst);
        crate::settings_window::converters_installing(&app, false);
        show(&app);
        if pm.is_none() {
            if !wanted.is_empty() {
                no_package_manager(&app, &format!("install {}", names(&wanted)));
            }
        } else if let Some((c, e)) = failed {
            crate::notify(&app, &format!("Couldn't install {}", c.name), &e);
        } else if !done.is_empty() {
            let verb = if done.len() == 1 { "is" } else { "are" };
            crate::notify(&app, &format!("{} {verb} installed", names(&done)), "Drop the files on the wheel again.");
        }
    });
}

/// Without winget or Homebrew nothing can be installed or updated (`doing`, such as
/// "install ffmpeg"): say where to get it, and open that page.
fn no_package_manager(app: &AppHandle, doing: &str) {
    log::line(&format!("{PACKAGE_MANAGER} not found; can't {doing}"));
    let title = format!("{PACKAGE_MANAGER} is needed to {doing}");
    if cfg!(target_os = "macos") {
        crate::notify(app, &title, "Install Homebrew from brew.sh (opening now), then try again.");
        open_page("/usr/bin/open", "https://brew.sh");
    } else if cfg!(windows) {
        crate::notify(app, &title, "winget comes with App Installer: the Microsoft Store opens on it now. Install or update it, then try again.");
        let system = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| "C:\\Windows".into());
        open_page(&system.join("explorer.exe").to_string_lossy(), APP_INSTALLER_STORE);
    } else {
        crate::notify(app, &title, "Install it with your distribution's package manager.");
    }
}

/// App Installer's page in the Microsoft Store app.
const APP_INSTALLER_STORE: &str = "ms-windows-store://pdp/?ProductId=9NBLGGH4NNS1";

/// Opens a web or Store page with the system's own opener (explorer.exe hands a
/// Store link to the Store app). A fixed address, never one from a file.
fn open_page(opener: &str, page: &str) {
    let mut open = Command::new(opener);
    open.arg(page).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    if let Err(e) = open.spawn() {
        log::line(&format!("couldn't open {page}: {e}"));
    }
}

/// The package manager. On Windows without winget, one try to get it first: winget
/// comes with App Installer, which Windows registers for a new account only at its
/// first sign-in, in the background, so it can still be missing for a while.
fn get_package_manager() -> Option<PathBuf> {
    let found = package_manager();
    #[cfg(windows)]
    if found.is_none() {
        return register_app_installer();
    }
    found
}

/// Microsoft's own fix for a missing winget: register App Installer for this account.
/// The winget alias shows up a moment after.
#[cfg(windows)]
fn register_app_installer() -> Option<PathBuf> {
    log::line("winget not found; registering App Installer");
    let system = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| "C:\\Windows".into());
    let mut cmd = Command::new(system.join("System32\\WindowsPowerShell\\v1.0\\powershell.exe"));
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "Add-AppxPackage -RegisterByFamilyName -MainPackage Microsoft.DesktopAppInstaller_8wekyb3d8bbwe",
    ]);
    match run(cmd, "App Installer") {
        Ok((status, output)) => log::line(&format!("App Installer ({status}): {}", output.join(" | "))),
        Err(e) => log::line(&format!("couldn't register App Installer: {e}")),
    }
    for _ in 0..10 {
        if let Some(pm) = package_manager() {
            log::line("winget is there now");
            return Some(pm);
        }
        thread::sleep(Duration::from_millis(500));
    }
    None
}

#[cfg(target_os = "macos")]
fn package_manager() -> Option<PathBuf> {
    ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"].into_iter().map(PathBuf::from).find(|p| p.is_file())
}

/// winget is an app execution alias: a link that only Windows itself can follow, so it
/// is looked for as a link.
#[cfg(windows)]
fn package_manager() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
    let alias = local.join("Microsoft\\WindowsApps\\winget.exe");
    std::fs::symlink_metadata(&alias).is_ok().then_some(alias)
}

/// Linux: pkexec, which runs the package manager as root once the desktop's password
/// window has said yes. Both must be there: without pkexec there is no password
/// window to ask with.
#[cfg(not(any(target_os = "macos", windows)))]
fn package_manager() -> Option<PathBuf> {
    let pkexec = PathBuf::from("/usr/bin/pkexec");
    (pkexec.is_file() && Linux::here().is_some()).then_some(pkexec)
}

#[cfg(not(any(target_os = "macos", windows)))]
const APT_GET: &str = "/usr/bin/apt-get";

/// Whether the menu can offer to install a missing converter, or to update them,
/// here. macOS and Windows always offer it (without Homebrew or winget the menu says
/// how to get them); Linux only where pkexec and apt, dnf, pacman or zypper are.
pub(crate) fn available() -> bool {
    AVAILABLE || package_manager().is_some()
}

/// winget: the exact package from winget's own source, without its installer's
/// questions. Choosing Install in the menu is taking the package's terms, as typing
/// the command would be. Admin rights, where the package needs them, still ask.
fn install_args(c: &Converter) -> Vec<&'static str> {
    if cfg!(windows) {
        vec![
            "install",
            "--id",
            c.winget,
            "--exact",
            "--source",
            "winget",
            "--silent",
            "--accept-package-agreements",
            "--accept-source-agreements",
        ]
    } else if cfg!(target_os = "macos") {
        let mut args = vec!["install"];
        args.extend(c.brew.split_whitespace());
        args
    } else {
        Linux::here().map(|pm| pm.install(c)).unwrap_or_default()
    }
}

/// Runs the package manager for `c`, and then checks that the app can find `c` where
/// a conversion would look: that, not the exit code, says whether it worked.
fn install_one(pm: &Path, c: &Converter) -> Result<(), String> {
    let mut cmd = Command::new(pm);
    cmd.args(install_args(c));
    #[cfg(target_os = "macos")]
    cmd.env("HOMEBREW_NO_ENV_HINTS", "1");
    let (status, output) = run(cmd, &PACKAGE_MANAGER.to_string())?;
    log::line(&format!("{PACKAGE_MANAGER} ({status}): {}", output.join(" | ")));
    if convert::installed(c) {
        return Ok(());
    }
    if status.success() {
        return Err(format!("{PACKAGE_MANAGER} finished, but {} still can't find {}. Show Log has the details.", crate::APP_NAME, c.name));
    }
    // winget's "already installed, and no newer version" (APPINSTALLER_CLI_ERROR_UPDATE_NOT_APPLICABLE).
    if cfg!(windows) && status.code() == Some(0x8A15_002B_u32 as i32) {
        return Err(format!("{PACKAGE_MANAGER} says {} is already installed, but {} can't find it. Show Log has the details.", c.name, crate::APP_NAME));
    }
    // pkexec: 126 = the password window was closed, 127 = no permission.
    if cfg!(target_os = "linux") && matches!(status.code(), Some(126 | 127)) {
        return Err("The admin password wasn't given, so nothing was installed.".into());
    }
    Err(stopped(status))
}

/// "winget stopped with code 0x8A150011. Show Log has what it said."
fn stopped(status: ExitStatus) -> String {
    let code = match status.code() {
        // winget's codes are Windows error numbers, which are written in hex.
        Some(code) if cfg!(windows) => format!(" with code {:#010X}", code as u32),
        Some(code) => format!(" with code {code}"),
        None => String::new(),
    };
    format!("{PACKAGE_MANAGER} stopped{code}. Show Log has what it said.")
}

/// What asking for a newer version of one converter came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    /// A newer version is installed now.
    Newer,
    /// There was nothing newer.
    Current,
    /// The package manager didn't install it (the converter came some other way), so
    /// it is left alone.
    NotOurs,
}

/// How "Update Converters…" went, converter by converter.
#[derive(Default)]
struct Report {
    newer: Vec<&'static Converter>,
    current: Vec<&'static Converter>,
    not_ours: Vec<&'static Converter>,
    failed: Vec<&'static Converter>,
    /// What went wrong with the first that failed.
    error: Option<String>,
}

impl Report {
    fn add(&mut self, c: &'static Converter, outcome: Result<Outcome, String>) {
        match outcome {
            Ok(Outcome::Newer) => self.newer.push(c),
            Ok(Outcome::Current) => self.current.push(c),
            Ok(Outcome::NotOurs) => self.not_ours.push(c),
            Err(e) => {
                log::line(&format!("couldn't update {}: {e}", c.name));
                self.failed.push(c);
                self.error.get_or_insert(e);
            }
        }
    }

    /// The notification's title and text.
    fn summary(&self) -> (String, String) {
        let mut lines = Vec::new();
        if !self.newer.is_empty() && !self.failed.is_empty() {
            lines.push(format!("Updated: {}.", names(&self.newer)));
        }
        if !self.current.is_empty() {
            lines.push(format!("Up to date: {}.", names(&self.current)));
        }
        if !self.not_ours.is_empty() {
            lines.push(format!("Not installed with {PACKAGE_MANAGER}, so left alone: {}.", names(&self.not_ours)));
        }
        let title = if !self.failed.is_empty() {
            lines.insert(0, self.error.clone().unwrap_or_default());
            format!("Couldn't update {}", names(&self.failed))
        } else if !self.newer.is_empty() {
            let verb = if self.newer.len() == 1 { "is" } else { "are" };
            format!("{} {verb} updated", names(&self.newer))
        } else if self.current.is_empty() && self.not_ours.is_empty() {
            return ("No converters to update".into(), "None of them is installed yet.".into());
        } else {
            "The converters are up to date".into()
        };
        (title, lines.join(" "))
    }
}

/// The converters this system's package manager provides. bsdtar is built into macOS
/// and Windows, and the app uses that one there.
fn updatable(c: &Converter) -> bool {
    c.tool != "bsdtar" || cfg!(not(any(target_os = "macos", windows)))
}

/// The menu's "Update Converters…": asks the package manager for a newer version of
/// each converter the app finds, on a thread of its own, and says what changed. One
/// package manager job at a time, shared with Install.
pub fn update(app: &AppHandle) {
    if INSTALLING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    thread::spawn(move || {
        crate::settings_window::converters_running(&app, true);
        let found: Vec<&'static Converter> = convert::CONVERTERS.iter().filter(|c| updatable(c) && convert::installed(c)).collect();
        log::line(&format!("updating converters: {}", names(&found)));
        let pm = get_package_manager();
        let report = pm.as_deref().map(|pm| update_all(pm, &found));
        INSTALLING.store(false, Ordering::SeqCst);
        crate::settings_window::converters_running(&app, false);
        show(&app);
        match report {
            None => no_package_manager(&app, "update the converters"),
            Some(report) => {
                let (title, text) = report.summary();
                log::line(&format!("{title}. {text}"));
                crate::notify(&app, &title, &text);
            }
        }
    });
}

/// macOS: `brew upgrade`, one converter after another. Homebrew only upgrades what it
/// installed, so the version it lists before and after says what happened.
#[cfg(target_os = "macos")]
fn update_all(pm: &Path, found: &[&'static Converter]) -> Report {
    let mut report = Report::default();
    for c in found {
        let outcome = match brew_version(pm, c) {
            None => Ok(Outcome::NotOurs),
            Some(before) => {
                let mut cmd = Command::new(pm);
                cmd.arg("upgrade").args(c.brew.split_whitespace()).env("HOMEBREW_NO_ENV_HINTS", "1");
                run(cmd, &PACKAGE_MANAGER.to_string()).and_then(|(status, output)| {
                    log::line(&format!("{PACKAGE_MANAGER} upgrade {} ({status}): {}", c.name, output.join(" | ")));
                    match brew_version(pm, c) {
                        Some(after) if after != before => Ok(Outcome::Newer),
                        _ if status.success() => Ok(Outcome::Current),
                        _ => Err(stopped(status)),
                    }
                })
            }
        };
        report.add(c, outcome);
    }
    report
}

/// "ffmpeg 9.0.2_1", or None when Homebrew didn't install it.
#[cfg(target_os = "macos")]
fn brew_version(pm: &Path, c: &Converter) -> Option<String> {
    let mut cmd = Command::new(pm);
    cmd.args(["list", "--versions"]).args(c.brew.split_whitespace()).env("HOMEBREW_NO_ENV_HINTS", "1");
    cmd.stdin(Stdio::null()).stderr(Stdio::null());
    let out = cmd.output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !text.is_empty()).then_some(text)
}

/// Windows: `winget upgrade`, one converter after another, from winget's own source
/// and without questions, as Install does.
#[cfg(windows)]
fn update_all(pm: &Path, found: &[&'static Converter]) -> Report {
    let mut report = Report::default();
    for c in found {
        let mut cmd = Command::new(pm);
        cmd.args([
            "upgrade",
            "--id",
            c.winget,
            "--exact",
            "--source",
            "winget",
            "--silent",
            "--accept-package-agreements",
            "--accept-source-agreements",
        ]);
        let outcome = run(cmd, &PACKAGE_MANAGER.to_string()).and_then(|(status, output)| {
            log::line(&format!("{PACKAGE_MANAGER} upgrade {} ({status}): {}", c.name, output.join(" | ")));
            winget_outcome(status.code(), &output).ok_or_else(|| stopped(status))
        });
        report.add(c, outcome);
    }
    report
}

/// What `winget upgrade` did, from its exit code and last lines; None when it failed.
#[cfg_attr(not(windows), allow(dead_code))]
fn winget_outcome(code: Option<i32>, output: &[String]) -> Option<Outcome> {
    // APPINSTALLER_CLI_ERROR_UPDATE_NOT_APPLICABLE and _NO_APPLICATIONS_FOUND.
    const NOTHING_NEWER: u32 = 0x8A15_002B;
    const NOT_INSTALLED: u32 = 0x8A15_0014;
    let said = |words: &str| output.iter().any(|l| l.contains(words));
    match code.map(|c| c as u32) {
        Some(NOT_INSTALLED) => Some(Outcome::NotOurs),
        _ if said("No installed package found") => Some(Outcome::NotOurs),
        Some(NOTHING_NEWER) => Some(Outcome::Current),
        Some(0) if said("No available upgrade found") || said("No newer package versions") => Some(Outcome::Current),
        Some(0) => Some(Outcome::Newer),
        _ => None,
    }
}

/// Linux: one upgrade for all of them, so the password is asked once: `apt-get install
/// --only-upgrade`, `dnf upgrade` or `zypper update`, from the package lists the
/// system refreshed last. The installed version before and after says what changed.
/// Arch updates as a whole (pacman -Syu); upgrading single packages there would be a
/// partial upgrade, so the report says to update the system instead.
#[cfg(not(any(target_os = "macos", windows)))]
fn update_all(pm: &Path, found: &[&'static Converter]) -> Report {
    let mut report = Report::default();
    let Some(linux) = Linux::here() else {
        return report;
    };
    let mut ours = Vec::new();
    for c in found {
        match installed_package_version(linux, linux.package(c)) {
            Some(before) => ours.push((*c, before)),
            None => report.add(c, Ok(Outcome::NotOurs)),
        }
    }
    if ours.is_empty() {
        return report;
    }
    if linux == Linux::Pacman {
        for (c, _) in ours {
            report.add(c, Err("Arch updates converters with the whole system: sudo pacman -Syu".into()));
        }
        return report;
    }
    let mut cmd = Command::new(pm);
    cmd.args(match linux {
        Linux::Apt => vec!["env", "DEBIAN_FRONTEND=noninteractive", APT_GET, "install", "--only-upgrade", "-y"],
        Linux::Dnf => vec!["/usr/bin/dnf", "upgrade", "-y"],
        _ => vec!["/usr/bin/zypper", "--non-interactive", "update"],
    });
    cmd.args(ours.iter().map(|(c, _)| linux.package(c)));
    let ran = run(cmd, &PACKAGE_MANAGER.to_string());
    if let Ok((status, output)) = &ran {
        log::line(&format!("{PACKAGE_MANAGER} upgrade ({status}): {}", output.join(" | ")));
    }
    for (c, before) in ours {
        let changed = installed_package_version(linux, linux.package(c)).is_some_and(|after| after != before);
        let outcome = match &ran {
            _ if changed => Ok(Outcome::Newer),
            Ok((status, _)) if ran_through(linux, status) => Ok(Outcome::Current),
            Ok((status, _)) if matches!(status.code(), Some(126 | 127)) => Err("The admin password wasn't given, so nothing was updated.".into()),
            Ok((status, _)) => Err(stopped(*status)),
            Err(e) => Err(e.clone()),
        };
        report.add(c, outcome);
    }
    report
}

/// Whether the package manager ran to the end: zypper also ends with 100 and up to say
/// something more (updates waiting, a restart needed).
#[cfg(not(any(target_os = "macos", windows)))]
fn ran_through(linux: Linux, status: &ExitStatus) -> bool {
    status.success() || (linux == Linux::Zypper && status.code().is_some_and(|c| c >= 100))
}

/// The installed version of `package`, or None when it isn't installed: dpkg on apt
/// systems, rpm on dnf and zypper ones (by what a package provides, as Fedora renames
/// some), pacman on Arch.
#[cfg(not(any(target_os = "macos", windows)))]
fn installed_package_version(linux: Linux, package: &str) -> Option<String> {
    let query = |program: &str, args: &[&str]| -> Option<String> {
        let out = Command::new(program).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (out.status.success() && !text.is_empty()).then_some(text)
    };
    match linux {
        Linux::Apt => dpkg_version(package),
        Linux::Pacman => query("/usr/bin/pacman", &["-Q", package]),
        Linux::Dnf | Linux::Zypper => query("/usr/bin/rpm", &["-q", "--whatprovides", package, "--qf", "%{NAME}-%{VERSION}-%{RELEASE}\\n"]),
    }
}

/// The installed version of a Debian package, or None when it isn't installed.
#[cfg(not(any(target_os = "macos", windows)))]
fn dpkg_version(package: &str) -> Option<String> {
    let out = Command::new("/usr/bin/dpkg-query")
        .args(["-W", "-f=${Status}|${Version}", package])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    installed_version(&String::from_utf8_lossy(&out.stdout))
}

/// dpkg-query's "install ok installed|7:6.1.1" gives "7:6.1.1"; a package removed but
/// with its settings kept ("deinstall ok config-files|…") gives None.
#[cfg_attr(any(target_os = "macos", windows), allow(dead_code))]
fn installed_version(text: &str) -> Option<String> {
    let (status, version) = text.trim().split_once('|')?;
    (status.ends_with(" installed") && !version.is_empty()).then(|| version.to_string())
}

/// Runs a package manager (or, on Windows, PowerShell: `what` names it) to the end or
/// LIMIT, both its outputs into one temporary file (not a pipe, which a chatty tool
/// could fill and stall on). Returns how it ended and its last lines.
fn run(mut cmd: Command, what: &str) -> Result<(ExitStatus, Vec<String>), String> {
    let mut out = tempfile::tempfile().map_err(|e| format!("work file: {e}"))?;
    let stdout = out.try_clone().map_err(|e| format!("work file: {e}"))?;
    let stderr = out.try_clone().map_err(|e| format!("work file: {e}"))?;
    cmd.stdin(Stdio::null()).stdout(stdout).stderr(stderr);
    convert::quiet(&mut cmd);
    // A group of its own, so a timeout stops what it started too.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn().map_err(|e| format!("could not start {what}: {e}"))?;
    let deadline = Instant::now() + LIMIT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(500)),
            Ok(None) => {
                convert::stop(&mut child);
                return Err(format!("{what} took longer than {} min and was stopped", LIMIT.as_secs() / 60));
            }
            Err(e) => {
                convert::stop(&mut child);
                return Err(format!("{what}: {e}"));
            }
        }
    };
    let len = out.seek(SeekFrom::End(0)).unwrap_or(0);
    let _ = out.seek(SeekFrom::Start(len.saturating_sub(8 * 1024)));
    let mut bytes = Vec::new();
    let _ = out.read_to_end(&mut bytes);
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    Ok((status, last_lines(&String::from_utf8_lossy(&bytes), home.as_deref())))
}

/// How many of the package manager's last lines go to the log.
const LOG_LINES: usize = 12;

/// The last lines a package manager wrote, for the log: progress bars redrawn with
/// carriage returns and colour codes taken out, and the home folder (which names the
/// user) written as "~".
fn last_lines(text: &str, home: Option<&Path>) -> Vec<String> {
    let mut plain = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            // An escape sequence such as "\x1b[2K" or "\x1b[0m": up to its final letter.
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else if ch == '\r' {
            plain.push('\n');
        } else if !ch.is_control() || ch == '\n' {
            plain.push(ch);
        }
    }
    if let Some(home) = home.map(|h| h.display().to_string()).filter(|h| h.len() > 1) {
        plain = plain.replace(&home, "~");
    }
    let lines: Vec<String> = plain
        .lines()
        .map(str::trim)
        // A bare progress bar or spinner says nothing once it has stopped.
        .filter(|l| l.chars().any(char::is_alphanumeric))
        .map(String::from)
        .collect();
    lines[lines.len().saturating_sub(LOG_LINES)..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(tool: &str) -> &'static Converter {
        convert::CONVERTERS.iter().find(|c| c.tool == tool).unwrap()
    }

    #[test]
    fn several_converters_are_named_in_one_line() {
        let (ffmpeg, pandoc, mupdf) = (named("ffmpeg"), named("pandoc"), named("mutool"));
        assert_eq!(names(&[ffmpeg]), "ffmpeg");
        assert_eq!(names(&[ffmpeg, mupdf]), "ffmpeg and MuPDF");
        assert_eq!(names(&[ffmpeg, pandoc, mupdf]), "ffmpeg, pandoc and MuPDF");
    }

    #[test]
    fn the_message_points_at_the_menu() {
        assert_eq!(
            pointer(named("soffice")),
            format!("LibreOffice is not installed. Choose Install LibreOffice in the {} menu.", crate::APP_NAME)
        );
    }

    #[test]
    fn each_system_asks_its_package_manager_for_the_exact_package() {
        if cfg!(windows) {
            let args = install_args(named("soffice"));
            assert_eq!(args[..5], ["install", "--id", "TheDocumentFoundation.LibreOffice", "--exact", "--source"]);
            assert!(args.contains(&"--silent"));
        } else if cfg!(target_os = "macos") {
            assert_eq!(install_args(named("soffice")), ["install", "--cask", "libreoffice"]);
            assert_eq!(install_args(named("mutool")), ["install", "mupdf-tools"]);
        }
        assert_eq!(Linux::Apt.install(named("mutool")), ["env", "DEBIAN_FRONTEND=noninteractive", "/usr/bin/apt-get", "install", "-y", "mupdf-tools"]);
        assert_eq!(Linux::Dnf.install(named("mutool")), ["/usr/bin/dnf", "install", "-y", "mupdf"]);
        assert_eq!(Linux::Pacman.install(named("pandoc")), ["/usr/bin/pacman", "-S", "--needed", "--noconfirm", "pandoc-cli"]);
        assert_eq!(Linux::Zypper.install(named("exiftool")), ["/usr/bin/zypper", "--non-interactive", "install", "perl-Image-ExifTool"]);
        assert_eq!(Linux::Dnf.hint(named("ffmpeg")), "sudo dnf install ffmpeg-free");
    }

    #[test]
    fn every_converter_has_a_package_on_every_linux() {
        for c in convert::CONVERTERS {
            for pm in Linux::ALL {
                assert!(!pm.package(c).is_empty(), "{} on {}", c.name, pm.name());
            }
        }
    }

    #[test]
    fn the_log_gets_the_last_plain_lines() {
        let text = "Found FFmpeg [Gyan.FFmpeg]\r\n\x1b[2K  ███▌      2.00 MB / 10 MB\r  ██████████  10 MB / 10 MB\r\n\
                    \x1b[32mSuccessfully installed\x1b[0m\r\n  -\r  \\\r\n\
                    Path environment variable modified: /Users/someone/AppData/Links\n";
        let lines = last_lines(text, Some(Path::new("/Users/someone")));
        assert_eq!(
            lines,
            [
                "Found FFmpeg [Gyan.FFmpeg]",
                "███▌      2.00 MB / 10 MB",
                "██████████  10 MB / 10 MB",
                "Successfully installed",
                "Path environment variable modified: ~/AppData/Links",
            ]
        );
        let many: String = (1..=40).map(|n| format!("line {n}\n")).collect();
        let lines = last_lines(&many, None);
        assert_eq!(lines.len(), LOG_LINES);
        assert_eq!(lines.last().map(String::as_str), Some("line 40"));
    }

    #[test]
    fn the_update_report_says_what_changed() {
        let (ffmpeg, pandoc, mupdf) = (named("ffmpeg"), named("pandoc"), named("mutool"));
        let mut r = Report::default();
        assert_eq!(r.summary().0, "No converters to update");
        r.add(ffmpeg, Ok(Outcome::Newer));
        r.add(pandoc, Ok(Outcome::Current));
        assert_eq!(r.summary(), ("ffmpeg is updated".into(), "Up to date: pandoc.".into()));
        r.add(mupdf, Ok(Outcome::NotOurs));
        assert_eq!(r.summary().1, format!("Up to date: pandoc. Not installed with {PACKAGE_MANAGER}, so left alone: MuPDF."));
        r.add(named("magick"), Err("it broke".into()));
        r.add(named("soffice"), Err("this too".into()));
        let (title, text) = r.summary();
        assert_eq!(title, "Couldn't update ImageMagick and LibreOffice");
        assert!(text.starts_with("it broke Updated: ffmpeg."), "{text}");
        let mut quiet = Report::default();
        quiet.add(pandoc, Ok(Outcome::Current));
        assert_eq!(quiet.summary().0, "The converters are up to date");
    }

    #[test]
    fn winget_upgrade_is_read_from_its_code_and_words() {
        let none: Vec<String> = Vec::new();
        assert_eq!(winget_outcome(Some(0), &none), Some(Outcome::Newer));
        assert_eq!(winget_outcome(Some(0x8A15_002B_u32 as i32), &none), Some(Outcome::Current));
        assert_eq!(winget_outcome(Some(0x8A15_0014_u32 as i32), &none), Some(Outcome::NotOurs));
        assert_eq!(winget_outcome(Some(0), &["No available upgrade found.".to_string()]), Some(Outcome::Current));
        assert_eq!(winget_outcome(Some(1), &["No installed package found matching input criteria.".to_string()]), Some(Outcome::NotOurs));
        assert_eq!(winget_outcome(Some(0x8A15_0011_u32 as i32), &none), None);
    }

    #[test]
    fn dpkg_says_which_version_is_installed() {
        assert_eq!(installed_version("install ok installed|7:6.1.1-3ubuntu5\n"), Some("7:6.1.1-3ubuntu5".into()));
        assert_eq!(installed_version("deinstall ok config-files|1.0"), None);
        assert_eq!(installed_version(""), None);
    }

    #[test]
    fn bsdtar_is_only_updated_where_the_system_does_not_bring_it() {
        assert!(updatable(named("ffmpeg")));
        assert_eq!(updatable(named("bsdtar")), cfg!(target_os = "linux"));
    }
}
