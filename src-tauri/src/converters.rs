//! Installs the converters that drops found missing, when the menu's "Install …" item
//! is chosen: with winget on Windows, with Homebrew on macOS. On Linux the setup script
//! installs them and apt needs an admin password, so there the message names the
//! command, as before.

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

#[cfg(target_os = "macos")]
const PACKAGE_MANAGER: &str = "Homebrew";
#[cfg(not(target_os = "macos"))]
const PACKAGE_MANAGER: &str = "winget";

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
    let Some(c) = convert::missing(&error).filter(|_| AVAILABLE) else {
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
    format!("{} is not installed. Choose Install {} in the Convert Wheel menu.", c.name, c.name)
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

/// The menu's Install item: installs the missing converters one after another, on a
/// thread of its own, and says how it went. It stops at the first that fails; that one
/// and the rest stay in the menu, to try again.
pub fn install(app: &AppHandle) {
    if INSTALLING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    thread::spawn(move || {
        let wanted = app.state::<Missing>().list().clone();
        let mut done: Vec<&Converter> = Vec::new();
        let mut failed: Option<(&Converter, String)> = None;
        let pm = package_manager();
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
        show(&app);
        if pm.is_none() {
            if !wanted.is_empty() {
                no_package_manager(&app, &wanted);
            }
        } else if let Some((c, e)) = failed {
            crate::notify(&app, &format!("Couldn't install {}", c.name), &e);
        } else if !done.is_empty() {
            let verb = if done.len() == 1 { "is" } else { "are" };
            crate::notify(&app, &format!("{} {verb} installed", names(&done)), "Drop the files on the wheel again.");
        }
    });
}

/// Without winget or Homebrew nothing can be installed: say where to get it.
fn no_package_manager(app: &AppHandle, wanted: &[&Converter]) {
    log::line(&format!("{PACKAGE_MANAGER} not found; can't install {}", names(wanted)));
    let title = format!("{PACKAGE_MANAGER} is needed to install {}", names(wanted));
    if cfg!(target_os = "macos") {
        crate::notify(app, &title, "Install Homebrew from brew.sh (opening now), then choose Install again.");
        let mut open = Command::new("/usr/bin/open");
        open.arg("https://brew.sh").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        if let Err(e) = open.status() {
            log::line(&format!("couldn't open brew.sh: {e}"));
        }
    } else {
        crate::notify(app, &title, "Install App Installer from the Microsoft Store, then choose Install again.");
    }
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

#[cfg(not(any(target_os = "macos", windows)))]
fn package_manager() -> Option<PathBuf> {
    None
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
    } else {
        let mut args = vec!["install"];
        args.extend(c.brew.split_whitespace());
        args
    }
}

/// Runs the package manager for `c`, and then checks that the app can find `c` where
/// a conversion would look: that, not the exit code, says whether it worked.
fn install_one(pm: &Path, c: &Converter) -> Result<(), String> {
    let mut cmd = Command::new(pm);
    cmd.args(install_args(c));
    #[cfg(target_os = "macos")]
    cmd.env("HOMEBREW_NO_ENV_HINTS", "1");
    let (status, output) = run(cmd)?;
    log::line(&format!("{PACKAGE_MANAGER} ({status}): {}", output.join(" | ")));
    if convert::installed(c) {
        return Ok(());
    }
    if status.success() {
        return Err(format!("{PACKAGE_MANAGER} finished, but Convert Wheel still can't find {}. Show Log has the details.", c.name));
    }
    // winget's "already installed, and no newer version" (APPINSTALLER_CLI_ERROR_UPDATE_NOT_APPLICABLE).
    if cfg!(windows) && status.code() == Some(0x8A15_002B_u32 as i32) {
        return Err(format!("{PACKAGE_MANAGER} says {} is already installed, but Convert Wheel can't find it. Show Log has the details.", c.name));
    }
    let code = match status.code() {
        // winget's codes are Windows error numbers, which are written in hex.
        Some(code) if cfg!(windows) => format!(" with code {:#010X}", code as u32),
        Some(code) => format!(" with code {code}"),
        None => String::new(),
    };
    Err(format!("{PACKAGE_MANAGER} stopped{code}. Show Log has what it said."))
}

/// Runs a package manager to the end or LIMIT, both its outputs into one temporary
/// file (not a pipe, which a chatty tool could fill and stall on). Returns how it ended
/// and its last lines.
fn run(mut cmd: Command) -> Result<(ExitStatus, Vec<String>), String> {
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
    let mut child = cmd.spawn().map_err(|e| format!("could not start {PACKAGE_MANAGER}: {e}"))?;
    let deadline = Instant::now() + LIMIT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(500)),
            Ok(None) => {
                convert::stop(&mut child);
                return Err(format!("{PACKAGE_MANAGER} took longer than {} min and was stopped", LIMIT.as_secs() / 60));
            }
            Err(e) => {
                convert::stop(&mut child);
                return Err(format!("{PACKAGE_MANAGER}: {e}"));
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
            "LibreOffice is not installed. Choose Install LibreOffice in the Convert Wheel menu."
        );
    }

    #[test]
    fn each_system_asks_its_package_manager_for_the_exact_package() {
        if cfg!(windows) {
            let args = install_args(named("soffice"));
            assert_eq!(args[..5], ["install", "--id", "TheDocumentFoundation.LibreOffice", "--exact", "--source"]);
            assert!(args.contains(&"--silent"));
        } else {
            assert_eq!(install_args(named("soffice")), ["install", "--cask", "libreoffice"]);
            assert_eq!(install_args(named("mutool")), ["install", "mupdf-tools"]);
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
}
