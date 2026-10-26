//! The Actions wheel (Control+Shift while dragging): what can be done with the dragged
//! files themselves. None of them changes a file: Copy Path puts the paths on the
//! clipboard, Show in Folder opens the file manager on them, Terminal opens one in
//! their folder, and AirDrop (macOS) offers to send them.

use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(not(windows))]
use std::process::Stdio;
use std::thread;

use tauri::AppHandle;

use crate::{log, notify, reveal};

/// Runs `action` on the dropped files. Nothing here waits on the thread that took the
/// drop: slow parts run on a thread of their own or on the main thread, as each
/// system wants.
pub fn run(app: &AppHandle, action: &str, paths: Vec<String>) {
    let files: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    log::line(&format!("{action} on {} file(s)", files.len()));
    match action {
        "act:path" => copy_paths(app, &files),
        "act:show" => reveal::show(app, files),
        "act:terminal" => {
            let app = app.clone();
            thread::spawn(move || {
                let Some(dir) = files.first().map(|f| folder_of(f)) else {
                    return;
                };
                if let Err(e) = open_terminal(&dir) {
                    log::line(&format!("couldn't open a terminal: {e}"));
                    notify(&app, "Couldn't open a terminal", &e);
                }
            });
        }
        #[cfg(target_os = "macos")]
        "act:airdrop" => airdrop(app, files),
        other => log::line(&format!("unknown action {other}")),
    }
}

/// The folder a terminal opens in: a dragged folder itself, or a file's folder.
fn folder_of(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().map(Path::to_path_buf).unwrap_or_else(|| path.to_path_buf())
    }
}

/// The paths, one per line, as they go on the clipboard.
fn paths_text(files: &[PathBuf]) -> String {
    files.iter().map(|f| f.display().to_string()).collect::<Vec<_>>().join("\n")
}

/// Copy Path, and a notification that says what was copied.
fn copy_paths(app: &AppHandle, files: &[PathBuf]) {
    let text = paths_text(files);
    let (title, body) = match files {
        [one] => ("Path copied".to_string(), one.display().to_string()),
        many => (format!("{} paths copied", many.len()), "One per line.".to_string()),
    };
    let app2 = app.clone();
    let done = move |result: Result<(), String>| match result {
        Ok(()) => notify(&app2, &title, &body),
        Err(e) => {
            log::line(&format!("couldn't copy the path: {e}"));
            notify(&app2, "Couldn't copy the path", &e);
        }
    };
    set_clipboard(app, text, done);
}

/// The clipboard belongs to the main thread on macOS and Linux.
#[cfg(target_os = "macos")]
fn set_clipboard(app: &AppHandle, text: String, done: impl FnOnce(Result<(), String>) + Send + 'static) {
    let _ = app.run_on_main_thread(move || {
        use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
        use objc2_foundation::NSString;

        let board = NSPasteboard::generalPasteboard();
        board.clearContents();
        let copied = board.setString_forType(&NSString::from_str(&text), unsafe { NSPasteboardTypeString });
        done(if copied { Ok(()) } else { Err("macOS didn't take the text".into()) });
    });
}

#[cfg(target_os = "linux")]
fn set_clipboard(app: &AppHandle, text: String, done: impl FnOnce(Result<(), String>) + Send + 'static) {
    let _ = app.run_on_main_thread(move || {
        let board = gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD);
        board.set_text(&text);
        // Keeps the text on the clipboard after the app quits, where a clipboard
        // manager runs.
        board.store();
        done(Ok(()));
    });
}

#[cfg(windows)]
fn set_clipboard(_app: &AppHandle, text: String, done: impl FnOnce(Result<(), String>) + Send + 'static) {
    thread::spawn(move || done(windows_clipboard(&text)));
}

/// Another app may hold the clipboard for a moment, so opening it is tried a few times.
#[cfg(windows)]
fn windows_clipboard(text: &str) -> Result<(), String> {
    use windows::Win32::Foundation::{GlobalFree, HANDLE};
    use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    use windows::Win32::System::Ole::CF_UNICODETEXT;

    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let mut opened = Err("the clipboard is busy".to_string());
    for _ in 0..10 {
        // SAFETY: plain Win32 call; closed again below.
        opened = unsafe { OpenClipboard(None) }.map_err(|e| e.to_string());
        if opened.is_ok() {
            break;
        }
        thread::sleep(std::time::Duration::from_millis(50));
    }
    opened?;
    // SAFETY: the clipboard is open on this thread. The memory is filled while locked
    // and, once SetClipboardData succeeds, belongs to the system; on a failure it is
    // freed here.
    let result = unsafe {
        (|| -> windows::core::Result<()> {
            EmptyClipboard()?;
            let mem = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2)?;
            let ptr = GlobalLock(mem) as *mut u16;
            if ptr.is_null() {
                let _ = GlobalFree(Some(mem));
                return Err(windows::core::Error::from_thread());
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
            // Reports an "error" once the lock count reaches zero, which is the point.
            let _ = GlobalUnlock(mem);
            if let Err(e) = SetClipboardData(u32::from(CF_UNICODETEXT.0), Some(HANDLE(mem.0))) {
                let _ = GlobalFree(Some(mem));
                return Err(e);
            }
            Ok(())
        })()
    };
    // SAFETY: opened above on this thread.
    let _ = unsafe { CloseClipboard() };
    result.map_err(|e| e.to_string())
}

/// Opens the system's terminal in `dir`. The folder goes in as a separate argument or
/// as the starting folder, never inside a command line, so no name can be read as a
/// command.
#[cfg(target_os = "macos")]
fn open_terminal(dir: &Path) -> Result<(), String> {
    let status = Command::new("/usr/bin/open")
        .args(["-a", "Terminal"])
        .arg(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Terminal didn't open ({status})"))
    }
}

/// PowerShell in a console of its own, started in the folder. (Windows Terminal reads
/// ";" in its arguments as a command separator, so it isn't handed the folder.)
#[cfg(windows)]
fn open_terminal(dir: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    let system = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| "C:\\Windows".into());
    Command::new(system.join("System32\\WindowsPowerShell\\v1.0\\powershell.exe"))
        .arg("-NoExit")
        .current_dir(dir)
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn()
        .map(drop)
        .map_err(|e| e.to_string())
}

/// The first terminal found, from the common desktops' own to Debian's default choice.
#[cfg(target_os = "linux")]
fn open_terminal(dir: &Path) -> Result<(), String> {
    let mut working_dir = std::ffi::OsString::from("--working-directory=");
    working_dir.push(dir);
    let candidates: [(&str, Vec<std::ffi::OsString>); 4] = [
        ("gnome-terminal", vec![working_dir.clone()]),
        ("kgx", vec![working_dir]),
        ("konsole", vec!["--workdir".into(), dir.into()]),
        ("x-terminal-emulator", Vec::new()),
    ];
    for (name, args) in candidates {
        let Some(path) = find_in_path(name) else {
            continue;
        };
        return Command::new(path)
            .args(args)
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(drop)
            .map_err(|e| e.to_string());
    }
    Err("no terminal app found (GNOME Terminal, Console, Konsole or x-terminal-emulator)".into())
}

#[cfg(target_os = "linux")]
fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

/// macOS: the AirDrop window for the files, in front, since the app itself has no
/// window to be in front of.
#[cfg(target_os = "macos")]
fn airdrop(app: &AppHandle, files: Vec<PathBuf>) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        use objc2::rc::Retained;
        use objc2::runtime::AnyObject;
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSApplication, NSSharingService, NSSharingServiceNameSendViaAirDrop};
        use objc2_foundation::{NSArray, NSString, NSURL};

        // As plain objects: the sharing service takes an untyped array.
        let urls: Vec<Retained<AnyObject>> = files
            .iter()
            .filter_map(|f| f.to_str())
            .map(|f| Retained::into_super(Retained::into_super(NSURL::fileURLWithPath(&NSString::from_str(f)))))
            .collect();
        let items: Retained<NSArray> = NSArray::from_retained_slice(&urls);
        let Some(service) = NSSharingService::sharingServiceNamed(unsafe { NSSharingServiceNameSendViaAirDrop }) else {
            notify(&handle, "AirDrop isn't available", "This Mac has no AirDrop.");
            return;
        };
        // SAFETY: an array of NSURLs, which is what AirDrop takes.
        if urls.is_empty() || !unsafe { service.canPerformWithItems(Some(&items)) } {
            notify(&handle, "Can't send with AirDrop", "AirDrop may be off, or these files can't be sent.");
            return;
        }
        if let Some(mtm) = MainThreadMarker::new() {
            #[allow(deprecated)]
            NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
        }
        // SAFETY: as above.
        unsafe { service.performWithItems(&items) };
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_go_one_per_line() {
        let files = [PathBuf::from("/tmp/a b.txt"), PathBuf::from("/tmp/c;d.pdf")];
        assert_eq!(paths_text(&files), "/tmp/a b.txt\n/tmp/c;d.pdf");
    }

    #[test]
    fn a_terminal_opens_in_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.txt");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(folder_of(&file), dir.path());
        assert_eq!(folder_of(dir.path()), dir.path());
    }
}
