//! The Actions wheel's Terminal: the terminal apps this computer has, for Settings, and
//! opening the chosen one in a folder. The folder always goes in as an argument of its
//! own, as the starting folder, or as a URL with the path encoded; never inside a
//! command line, so no folder name can be read as a command.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::log;

/// A terminal app Settings can offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Terminal {
    /// The name the settings file keeps.
    pub key: &'static str,
    /// The name Settings shows.
    pub name: &'static str,
}

/// How a terminal is told which folder to start in, after the arguments in `before`.
/// Not every system uses each way.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
enum Folder {
    /// An option, then the folder as an argument of its own: `--cwd <folder>`.
    After(&'static str),
    /// The folder joined to an option: `--working-directory=<folder>`.
    Joined(&'static str),
    /// Only as the starting folder of the new process.
    Start,
}

struct Known {
    key: &'static str,
    name: &'static str,
    /// macOS: the app's bundle id. Windows and Linux: the program's file name.
    id: &'static str,
    how: How,
}

/// Each system uses some of these: macOS the first three, Windows the last two, Linux
/// Args.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
enum How {
    /// macOS: `open -b <id> <folder>`; the app opens a window in the folder it is given.
    OpenFolder,
    /// macOS: Warp's own link, warp://action/new_window?path=<folder>.
    WarpLink,
    /// Started with these arguments (macOS: `open -na <app> --args ...`, a new copy).
    Args(&'static [&'static str], Folder),
    /// Windows: a console program in a console of its own, started in the folder.
    Console(&'static [&'static str]),
}

#[cfg(target_os = "macos")]
const KNOWN: &[Known] = &[
    Known { key: "terminal", name: "Terminal", id: "com.apple.Terminal", how: How::OpenFolder },
    Known { key: "iterm", name: "iTerm2", id: "com.googlecode.iterm2", how: How::OpenFolder },
    Known { key: "warp", name: "Warp", id: "dev.warp.Warp-Stable", how: How::WarpLink },
    // Every Ghostty setting is also a command-line option.
    Known { key: "ghostty", name: "Ghostty", id: "com.mitchellh.ghostty", how: How::Args(&[], Folder::Joined("--working-directory=")) },
    Known { key: "kitty", name: "kitty", id: "net.kovidgoyal.kitty", how: How::Args(&[], Folder::After("--directory")) },
    Known { key: "alacritty", name: "Alacritty", id: "org.alacritty", how: How::Args(&[], Folder::After("--working-directory")) },
    Known { key: "wezterm", name: "WezTerm", id: "com.github.wez.wezterm", how: How::Args(&["start"], Folder::After("--cwd")) },
];

#[cfg(windows)]
const KNOWN: &[Known] = &[
    Known { key: "powershell", name: "Windows PowerShell", id: "powershell.exe", how: How::Console(&["-NoExit"]) },
    Known { key: "pwsh", name: "PowerShell 7", id: "pwsh.exe", how: How::Console(&["-NoExit"]) },
    // Windows Terminal reads ";" in its arguments as a command separator, so it gets
    // "." and starts in the folder rather than being handed its name.
    Known { key: "wt", name: "Windows Terminal", id: "wt.exe", how: How::Args(&["-d", "."], Folder::Start) },
    Known { key: "cmd", name: "Command Prompt", id: "cmd.exe", how: How::Console(&["/K"]) },
    Known { key: "gitbash", name: "Git Bash", id: "git-bash.exe", how: How::Args(&[], Folder::Joined("--cd=")) },
];

/// The first of these the system has is the default (GNOME's, then KDE's, then
/// Debian's chosen one); the rest are offered in Settings.
#[cfg(target_os = "linux")]
const KNOWN: &[Known] = &[
    Known { key: "gnome-terminal", name: "GNOME Terminal", id: "gnome-terminal", how: How::Args(&[], Folder::Joined("--working-directory=")) },
    Known { key: "kgx", name: "Console", id: "kgx", how: How::Args(&[], Folder::Joined("--working-directory=")) },
    Known { key: "ptyxis", name: "Ptyxis", id: "ptyxis", how: How::Args(&["--new-window"], Folder::Joined("--working-directory=")) },
    Known { key: "konsole", name: "Konsole", id: "konsole", how: How::Args(&[], Folder::After("--workdir")) },
    Known { key: "x-terminal-emulator", name: "Debian's default terminal", id: "x-terminal-emulator", how: How::Args(&[], Folder::Start) },
    Known { key: "xfce4-terminal", name: "Xfce Terminal", id: "xfce4-terminal", how: How::Args(&[], Folder::Joined("--working-directory=")) },
    Known { key: "mate-terminal", name: "MATE Terminal", id: "mate-terminal", how: How::Args(&[], Folder::Joined("--working-directory=")) },
    Known { key: "tilix", name: "Tilix", id: "tilix", how: How::Args(&[], Folder::Joined("--working-directory=")) },
    Known { key: "terminator", name: "Terminator", id: "terminator", how: How::Args(&[], Folder::Joined("--working-directory=")) },
    Known { key: "kitty", name: "kitty", id: "kitty", how: How::Args(&[], Folder::After("--directory")) },
    Known { key: "alacritty", name: "Alacritty", id: "alacritty", how: How::Args(&[], Folder::After("--working-directory")) },
    Known { key: "wezterm", name: "WezTerm", id: "wezterm", how: How::Args(&["start"], Folder::After("--cwd")) },
    Known { key: "ghostty", name: "Ghostty", id: "ghostty", how: How::Args(&[], Folder::Joined("--working-directory=")) },
    Known { key: "foot", name: "foot", id: "foot", how: How::Args(&[], Folder::Joined("--working-directory=")) },
];

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
const KNOWN: &[Known] = &[];

/// The terminals this computer has, in the order Settings lists them.
pub fn installed() -> Vec<Terminal> {
    KNOWN.iter().filter(|k| find(k).is_some()).map(|k| Terminal { key: k.key, name: k.name }).collect()
}

/// Whether `key` names a terminal the app knows on this system (installed or not).
pub fn known(key: &str) -> bool {
    KNOWN.iter().any(|k| k.key == key)
}

/// Opens `choice` (a key from `installed`) in `dir`. With None, or when the chosen one
/// is gone, the system's own terminal opens: Terminal on macOS, Windows PowerShell, or
/// the first Linux terminal found.
pub fn open(choice: Option<&str>, dir: &Path) -> Result<(), String> {
    let chosen = choice.and_then(|key| KNOWN.iter().find(|k| k.key == key)).and_then(|k| find(k).map(|at| (k, at)));
    if let (Some(key), None) = (choice, &chosen) {
        log::line(&format!("terminal {key} not found; the system's own opens instead"));
    }
    let (known, at) = chosen.or_else(|| KNOWN.iter().find_map(|k| find(k).map(|at| (k, at)))).ok_or_else(none_found)?;
    log::line(&format!("opening {} in a folder", known.name));
    launch(known, &at, dir).map_err(|e| format!("{} didn't open: {e}", known.name))
}

fn none_found() -> String {
    if cfg!(target_os = "linux") {
        "no terminal app found (GNOME Terminal, Console, Ptyxis, Konsole or x-terminal-emulator)".into()
    } else {
        "no terminal app found".into()
    }
}

/// The arguments that name the folder for `how`.
fn folder_args(folder: Folder, dir: &Path) -> Vec<OsString> {
    match folder {
        Folder::After(option) => vec![option.into(), dir.into()],
        Folder::Joined(option) => {
            let mut joined = OsString::from(option);
            joined.push(dir);
            vec![joined]
        }
        Folder::Start => Vec::new(),
    }
}

/// warp://action/new_window?path=..., the folder's bytes percent-encoded except the
/// letters, digits, "-._~" and "/".
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn warp_link(dir: &Path) -> String {
    let mut link = String::from("warp://action/new_window?path=");
    for byte in dir.as_os_str().as_encoded_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => link.push(char::from(*byte)),
            other => link.push_str(&format!("%{other:02X}")),
        }
    }
    link
}

#[cfg(target_os = "macos")]
fn find(k: &Known) -> Option<PathBuf> {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::NSString;
    let url = NSWorkspace::sharedWorkspace().URLForApplicationWithBundleIdentifier(&NSString::from_str(k.id))?;
    url.path().map(|p| PathBuf::from(p.to_string())).filter(|p| p.is_dir())
}

#[cfg(target_os = "macos")]
fn launch(k: &Known, app: &Path, dir: &Path) -> Result<(), String> {
    let mut cmd = Command::new("/usr/bin/open");
    match k.how {
        How::OpenFolder => {
            cmd.args(["-b", k.id]).arg(dir);
        }
        How::WarpLink => {
            cmd.arg(warp_link(dir));
        }
        // A new copy of the app, which reads its arguments only when it starts.
        How::Args(before, folder) => {
            cmd.arg("-na").arg(app).arg("--args").args(before).args(folder_args(folder, dir));
        }
        How::Console(_) => return Err("not a macOS terminal".into()),
    }
    let status = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("macOS couldn't start it ({status})"))
    }
}

#[cfg(windows)]
fn find(k: &Known) -> Option<PathBuf> {
    let env = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let system = env("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()).join("System32");
    let programs = env("ProgramFiles").unwrap_or_else(|| "C:\\Program Files".into());
    let candidates: Vec<PathBuf> = match k.key {
        "powershell" => vec![system.join("WindowsPowerShell\\v1.0\\powershell.exe")],
        "cmd" => vec![system.join("cmd.exe")],
        "pwsh" => in_path(k.id).into_iter().chain([programs.join("PowerShell\\7\\pwsh.exe")]).collect(),
        // An app execution alias: a link only Windows itself can follow, so it is
        // looked at without following it.
        "wt" => env("LOCALAPPDATA").map(|l| l.join("Microsoft\\WindowsApps\\wt.exe")).into_iter().collect(),
        "gitbash" => vec![programs.join("Git\\git-bash.exe")],
        _ => Vec::new(),
    };
    candidates.into_iter().find(|p| std::fs::symlink_metadata(p).is_ok())
}

#[cfg(windows)]
fn launch(k: &Known, program: &Path, dir: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    let mut cmd = Command::new(program);
    cmd.current_dir(dir);
    match k.how {
        How::Console(args) => {
            cmd.args(args).creation_flags(CREATE_NEW_CONSOLE);
        }
        How::Args(before, folder) => {
            cmd.args(before).args(folder_args(folder, dir));
        }
        _ => return Err("not a Windows terminal".into()),
    }
    cmd.spawn().map(drop).map_err(|e| e.to_string())
}

#[cfg(target_os = "linux")]
fn find(k: &Known) -> Option<PathBuf> {
    in_path(k.id)
}

#[cfg(target_os = "linux")]
fn launch(k: &Known, program: &Path, dir: &Path) -> Result<(), String> {
    let How::Args(before, folder) = k.how else {
        return Err("not a Linux terminal".into());
    };
    Command::new(program)
        .args(before)
        .args(folder_args(folder, dir))
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|e| e.to_string())
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
fn find(_k: &Known) -> Option<PathBuf> {
    None
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
fn launch(_k: &Known, _program: &Path, _dir: &Path) -> Result<(), String> {
    Err("no terminal here".into())
}

/// A program in an absolute folder of the PATH.
#[cfg(any(windows, target_os = "linux"))]
fn in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).filter(|dir| dir.is_absolute()).map(|dir| dir.join(name)).find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_unique_and_the_system_one_is_known() {
        let mut keys: Vec<&str> = KNOWN.iter().map(|k| k.key).collect();
        keys.sort_unstable();
        let count = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), count);
        if cfg!(target_os = "macos") {
            assert!(known("terminal") && known("iterm"));
            // Terminal comes with macOS.
            assert_eq!(installed().first().map(|t| t.key), Some("terminal"));
        }
        if cfg!(windows) {
            assert!(known("powershell") && known("wt"));
        }
        assert!(!known("bash; rm -rf ~"));
    }

    #[test]
    fn the_folder_is_always_its_own_argument() {
        let dir = Path::new("/tmp/a b;c");
        assert_eq!(folder_args(Folder::After("--cwd"), dir), [OsString::from("--cwd"), OsString::from("/tmp/a b;c")]);
        assert_eq!(folder_args(Folder::Joined("--working-directory="), dir), [OsString::from("--working-directory=/tmp/a b;c")]);
        assert!(folder_args(Folder::Start, dir).is_empty());
    }

    #[test]
    fn warp_gets_the_folder_encoded() {
        assert_eq!(warp_link(Path::new("/Users/me/My Files/a&b?")), "warp://action/new_window?path=/Users/me/My%20Files/a%26b%3F");
        assert_eq!(warp_link(Path::new("/tmp/é")), "warp://action/new_window?path=/tmp/%C3%A9");
    }
}
