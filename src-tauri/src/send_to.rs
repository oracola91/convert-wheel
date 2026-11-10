//! Windows: "Send to > Convert Wheel" in File Explorer's right-click menu, for the
//! keyboard wheel (council 2026-11-10, keyboard mode E). A shortcut to this copy of
//! the app in the user's own SendTo folder: Explorer starts the app with the selected
//! files, the running copy gets them (single instance) and opens the wheel on them.
//! On unless switched off in Settings; nothing outside the user's own profile.

use std::path::{Path, PathBuf};

use windows::core::{Interface, HSTRING};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

/// %APPDATA%\Microsoft\Windows\SendTo\Convert Wheel.lnk
fn shortcut() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from)?;
    Some(appdata.join("Microsoft\\Windows\\SendTo").join(format!("{}.lnk", crate::APP_NAME)))
}

/// Puts the Send To shortcut in place, pointing at this copy of the app (again on each
/// start, so it follows an update to another folder), or takes it away.
pub fn set(on: bool) {
    let Some(link) = shortcut() else {
        return;
    };
    if !on {
        if link.exists() {
            if let Err(e) = std::fs::remove_file(&link) {
                crate::log::line(&format!("couldn't remove the Send To entry: {e}"));
            }
        }
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    if let Err(e) = write(&link, &exe) {
        crate::log::line(&format!("couldn't make the Send To entry: {e}"));
    }
}

fn write(link: &Path, exe: &Path) -> windows::core::Result<()> {
    unsafe {
        // SAFETY: COM is set up on this thread for this call alone and taken down
        // after it; the shell link object lives only inside it.
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let made = (|| {
            let shell_link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            shell_link.SetPath(&HSTRING::from(exe))?;
            shell_link.SetDescription(&HSTRING::from("Open the wheel on these files"))?;
            shell_link.SetIconLocation(&HSTRING::from(exe), 0)?;
            let file: IPersistFile = shell_link.cast()?;
            file.Save(&HSTRING::from(link), true)
        })();
        if com.is_ok() {
            CoUninitialize();
        }
        made
    }
}
