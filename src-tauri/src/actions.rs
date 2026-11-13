//! The Actions wheel (Control+Shift while dragging): what can be done with the dragged
//! files themselves. None of them changes a file: Copy Path puts the paths on the
//! clipboard, Show in Folder opens the file manager on them, Copy To and Move To put
//! them in a folder of the user's choosing (transfer.rs), Terminal opens the one chosen
//! in Settings in their folder (terminal.rs), and AirDrop (macOS) or Share offers to
//! send them (macOS: a menu of the
//! services macOS has for the files; Windows: the share sheet; Linux: email,
//! Bluetooth or a phone through KDE Connect, whichever this computer has). Making and unpacking archives take
//! a while, so they go through the conversion queue (archive.rs).

use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::process::{Command, Stdio};
use std::thread;

use tauri::AppHandle;

use crate::{log, notify, reveal};

/// Runs `action` on the dropped files. Nothing here waits on the thread that took the
/// drop: slow parts run on a thread of their own or on the main thread, as each
/// system wants.
pub fn run(app: &AppHandle, action: &str, paths: Vec<String>) {
    if matches!(action, "act:zip" | "act:tgz" | "act:tar" | "act:unpack") {
        crate::enqueue(app, paths, action.to_string());
        return;
    }
    let files: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    log::line(&format!("{action} on {} file(s)", files.len()));
    match action {
        "act:path" => copy_paths(app, &files),
        "act:show" => reveal::show(app, files),
        "act:copyto" => crate::transfer::start(app, false, files),
        "act:moveto" => crate::transfer::start(app, true, files),
        "act:terminal" => {
            let app = app.clone();
            // The terminal chosen in Settings, or the system's own.
            let choice = {
                use tauri::Manager;
                app.state::<crate::settings::Store>().get().terminal
            };
            thread::spawn(move || {
                let Some(dir) = files.first().map(|f| folder_of(f)) else {
                    return;
                };
                if let Err(e) = crate::terminal::open(choice.as_deref(), &dir) {
                    log::line(&format!("couldn't open a terminal: {e}"));
                    notify(&app, "Couldn't open a terminal", &e);
                }
            });
        }
        #[cfg(target_os = "macos")]
        "act:airdrop" => airdrop(app, files),
        #[cfg(target_os = "macos")]
        "act:float" => crate::float_card::open(app, files),
        #[cfg(not(target_os = "macos"))]
        "act:float" => crate::float_web::open(app, files),
        "act:share" => share(app, files),
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

/// Puts `text` on the clipboard for a tool that says itself what it did (Read QR Code).
pub(crate) fn copy_text(app: &AppHandle, text: String) {
    set_clipboard(app, text, |result| {
        if let Err(e) = result {
            log::line(&format!("couldn't copy the text: {e}"));
        }
    });
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

/// macOS: Share. A menu at the pointer with what macOS offers for these files (Mail,
/// Messages, AirDrop, Notes and the apps that add their own), the list Finder's Share
/// shows.
#[cfg(target_os = "macos")]
fn share(app: &AppHandle, files: Vec<PathBuf>) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || mac_share::show(&handle, &files));
}

#[cfg(target_os = "macos")]
mod mac_share {
    use std::cell::Cell;
    use std::path::PathBuf;

    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject};
    use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSApplication, NSEvent, NSMenu, NSMenuItem, NSSharingService};
    use objc2_foundation::{NSArray, NSString, NSURL};
    use tauri::AppHandle;

    use crate::{log, notify};

    thread_local! {
        /// The service picked in the menu, by its place in the list.
        static PICKED: Cell<Option<isize>> = const { Cell::new(None) };
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "PirouetteShareMenuTarget"]
        struct Target;

        impl Target {
            #[unsafe(method(pick:))]
            fn pick(&self, item: &NSMenuItem) {
                PICKED.with(|p| p.set(Some(item.tag())));
            }
        }
    );

    impl Target {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(());
            // SAFETY: NSObject's plain init.
            unsafe { msg_send![super(this), init] }
        }
    }

    /// Main thread. The menu runs until a pick or a click elsewhere.
    pub fn show(app: &AppHandle, files: &[PathBuf]) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let urls: Vec<Retained<AnyObject>> = files
            .iter()
            .filter_map(|f| f.to_str())
            .map(|f| Retained::into_super(Retained::into_super(NSURL::fileURLWithPath(&NSString::from_str(f)))))
            .collect();
        if urls.is_empty() {
            return;
        }
        let items: Retained<NSArray> = NSArray::from_retained_slice(&urls);
        // The menu-building form of the list; NSSharingServicePicker only shows its
        // own popover from a mouse-down, and the wheel's drop is a mouse-up.
        // SAFETY: an array of NSURLs.
        #[allow(deprecated)]
        let services = unsafe { NSSharingService::sharingServicesForItems(&items) };
        if services.count() == 0 {
            notify(app, "Nothing to share with", "macOS offers no way to share these files.");
            return;
        }
        let target = Target::new(mtm);
        let target_object: &AnyObject = &target;
        let menu = NSMenu::new(mtm);
        for (i, service) in services.iter().enumerate() {
            // SAFETY: `pick:` is Target's method, taking the menu item.
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &service.menuItemTitle(), Some(sel!(pick:)), &NSString::new())
            };
            // SAFETY: `target` outlives the menu, which runs to the end below.
            unsafe { item.setTarget(Some(target_object)) };
            item.setTag(i as isize);
            item.setImage(Some(&service.image()));
            menu.addItem(&item);
        }
        #[allow(deprecated)]
        NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
        PICKED.with(|p| p.set(None));
        menu.popUpMenuPositioningItem_atLocation_inView(None, NSEvent::mouseLocation(), None);
        let Some(i) = PICKED.with(Cell::take) else {
            return;
        };
        if let Some(service) = usize::try_from(i).ok().and_then(|i| services.iter().nth(i)) {
            log::line(&format!("share with {}", service.menuItemTitle()));
            // SAFETY: the same NSURLs the service was listed for.
            unsafe { service.performWithItems(&items) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_go_one_per_line() {
        let files = [PathBuf::from("/tmp/a b.txt"), PathBuf::from("/tmp/c;d.pdf")];
        assert_eq!(paths_text(&files), "/tmp/a b.txt\n/tmp/c;d.pdf");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn kde_connect_lines_give_phones() {
        assert_eq!(phone_line("a1b2_c3 Pixel 8 Pro"), Some(("a1b2_c3".into(), "Pixel 8 Pro".into())));
        assert_eq!(phone_line("no-name"), None);
        assert_eq!(phone_line("bad;id Phone"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn each_phone_gets_every_file() {
        let files = [PathBuf::from("/tmp/a.jpg"), PathBuf::from("/tmp/b.pdf")];
        let phones = vec![("id1".to_string(), "Pixel".to_string())];
        let found = share_targets(&files, phones);
        if let Some(phone) = found.iter().find(|t| t.label == "Phone: Pixel") {
            let args: Vec<_> = phone.args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
            assert_eq!(args, ["--device", "id1", "--share", "/tmp/a.jpg", "--share", "/tmp/b.pdf"]);
        }
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

/// Windows: the system's share sheet (Nearby Share, Mail, apps), for the wheel's window.
#[cfg(windows)]
fn share(app: &AppHandle, files: Vec<PathBuf>) {
    use tauri::Manager;
    let Some(window) = app.get_webview_window("wheel") else {
        return;
    };
    let handle = app.clone();
    // The window handle is not Send: it is read on the main thread.
    let _ = app.run_on_main_thread(move || {
        let shown = window.hwnd().map_err(|e| windows::core::Error::new(windows::core::HRESULT(-1), e.to_string()));
        if let Err(e) = shown.and_then(|hwnd| share_sheet(hwnd, &files)) {
            log::line(&format!("couldn't open the share sheet: {e}"));
            notify(&handle, "Couldn't open Share", &e.message());
        }
    });
}

#[cfg(windows)]
fn share_sheet(hwnd: windows::Win32::Foundation::HWND, files: &[PathBuf]) -> windows::core::Result<()> {
    use windows::core::{factory, AgileReference, Interface, HSTRING};
    use windows::ApplicationModel::DataTransfer::{DataRequestedEventArgs, DataTransferManager};
    use windows_collections::IIterable;
    use windows::Foundation::TypedEventHandler;
    use windows::Storage::{IStorageItem, StorageFile};
    use windows::Win32::UI::Shell::IDataTransferManagerInterop;

    let mut items: Vec<Option<IStorageItem>> = Vec::new();
    for f in files {
        let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(f.as_os_str()))?.join()?;
        items.push(Some(file.cast()?));
    }
    let title = match files {
        [one] => one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        many => format!("{} files", many.len()),
    };
    // The share sheet asks for the files on a thread of its own: the list goes there
    // through an agile reference, as COM objects may not cross threads by themselves.
    let list: IIterable<IStorageItem> = items.into();
    let list = AgileReference::new(&list)?;
    let interop = factory::<DataTransferManager, IDataTransferManagerInterop>()?;
    let manager: DataTransferManager = unsafe { interop.GetForWindow(hwnd)? };
    // One share at a time: the handler answers only the request this share makes.
    let token = std::sync::Arc::new(std::sync::Mutex::new(None));
    let slot = token.clone();
    let handler = TypedEventHandler::new(move |sender: windows::core::Ref<DataTransferManager>, args: windows::core::Ref<DataRequestedEventArgs>| {
        if let Some(args) = args.as_ref() {
            let data = args.Request()?.Data()?;
            data.Properties()?.SetTitle(&HSTRING::from(title.as_str()))?;
            data.SetStorageItemsReadOnly(&list.resolve()?)?;
        }
        if let (Some(sender), Some(t)) = (sender.as_ref(), slot.lock().map(|g| *g).unwrap_or(None)) {
            let _ = sender.RemoveDataRequested(t);
        }
        Ok(())
    });
    let t = manager.DataRequested(&handler)?;
    if let Ok(mut g) = token.lock() {
        *g = Some(t);
    }
    unsafe { interop.ShowShareUIForWindow(hwnd) }
}

/// Linux has no share sheet of its own. What this computer has: a new email with the
/// files attached (xdg-email), Bluetooth (GNOME's or Blueman's send window), and each
/// phone KDE Connect (or GSConnect) can reach. One of them starts at once; with more,
/// a small window asks which.
#[cfg(target_os = "linux")]
fn share(app: &AppHandle, files: Vec<PathBuf>) {
    let app = app.clone();
    thread::spawn(move || {
        let found = share_targets(&files, kdeconnect_phones());
        match found.as_slice() {
            [] => {
                log::line("share: no email, Bluetooth or KDE Connect found");
                notify(&app, "Nothing to share with", "Install xdg-utils for email, or KDE Connect for a phone.");
            }
            [one] => start_share(&app, one),
            _ => {
                let handle = app.clone();
                let _ = app.run_on_main_thread(move || choose_share(&handle, found));
            }
        }
    });
}

/// One way to share: what the chooser shows, and the program that does it.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, PartialEq)]
struct ShareTarget {
    label: String,
    program: PathBuf,
    args: Vec<std::ffi::OsString>,
}

/// The ways this computer can share `files`, with `phones` (id, name) from KDE Connect.
#[cfg(target_os = "linux")]
fn share_targets(files: &[PathBuf], phones: Vec<(String, String)>) -> Vec<ShareTarget> {
    let mut found = Vec::new();
    if let Some(program) = find_in_path("xdg-email") {
        let args = files.iter().flat_map(|f| ["--attach".into(), f.clone().into_os_string()]).collect();
        found.push(ShareTarget { label: "Email".into(), program, args });
    }
    if let Some(program) = ["bluetooth-sendto", "blueman-sendto"].into_iter().find_map(find_in_path) {
        let args = files.iter().map(|f| f.clone().into_os_string()).collect();
        found.push(ShareTarget { label: "Bluetooth".into(), program, args });
    }
    if let Some(program) = find_in_path("kdeconnect-cli") {
        for (id, name) in phones {
            let mut args: Vec<std::ffi::OsString> = vec!["--device".into(), id.into()];
            for f in files {
                args.push("--share".into());
                args.push(f.clone().into_os_string());
            }
            found.push(ShareTarget { label: format!("Phone: {name}"), program: program.clone(), args });
        }
    }
    found
}

/// The phones KDE Connect can reach now, as (id, name). Empty without KDE Connect.
#[cfg(target_os = "linux")]
fn kdeconnect_phones() -> Vec<(String, String)> {
    let Some(cli) = find_in_path("kdeconnect-cli") else {
        return Vec::new();
    };
    let out = Command::new(cli).args(["--list-available", "--id-name-only"]).stdin(Stdio::null()).stderr(Stdio::null()).output();
    out.map(|o| String::from_utf8_lossy(&o.stdout).lines().filter_map(phone_line).collect()).unwrap_or_default()
}

/// "id name" from `kdeconnect-cli --id-name-only`.
#[cfg(target_os = "linux")]
fn phone_line(line: &str) -> Option<(String, String)> {
    let (id, name) = line.trim().split_once(' ')?;
    let ok = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    ok.then(|| (id.to_string(), name.trim().to_string()))
}

#[cfg(target_os = "linux")]
fn start_share(app: &AppHandle, target: &ShareTarget) {
    log::line(&format!("share with {}", target.label));
    let started = Command::new(&target.program).args(&target.args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    match started {
        Ok(mut child) => {
            let (handle, label) = (app.clone(), target.label.clone());
            thread::spawn(move || {
                if !child.wait().map(|s| s.success()).unwrap_or(false) {
                    log::line(&format!("share with {label} didn't work"));
                    notify(&handle, &format!("Couldn't share: {label}"), "For email, set a mail app as the default in Settings > Default Apps.");
                }
            });
        }
        Err(e) => {
            log::line(&format!("share with {}: {e}", target.label));
            notify(app, &format!("Couldn't share: {}", target.label), &e.to_string());
        }
    }
}

/// Asks which way to share, in a small window of the system's own look. Main thread.
#[cfg(target_os = "linux")]
fn choose_share(app: &AppHandle, found: Vec<ShareTarget>) {
    use gtk::prelude::*;
    let dialog = gtk::MessageDialog::new(
        None::<&gtk::Window>,
        gtk::DialogFlags::empty(),
        gtk::MessageType::Question,
        gtk::ButtonsType::None,
        "Share with",
    );
    dialog.set_title(crate::APP_NAME);
    dialog.set_keep_above(true);
    for (i, target) in found.iter().enumerate() {
        dialog.add_button(&target.label, gtk::ResponseType::Other(i as u16));
    }
    dialog.add_button("Cancel", gtk::ResponseType::Cancel);
    let app = app.clone();
    dialog.connect_response(move |d, response| {
        if let gtk::ResponseType::Other(i) = response {
            if let Some(target) = found.get(usize::from(i)) {
                start_share(&app, target);
            }
        }
        d.close();
    });
    dialog.present();
}
