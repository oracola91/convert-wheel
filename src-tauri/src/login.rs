//! Open at Login. macOS: the system's login items (System Settings > General >
//! Login Items); only an app in a bundle can be one, so the switch is off limits when
//! the app runs straight from Terminal. Windows: the Run key in the registry.
//! Linux: an XDG autostart entry.

#[cfg(target_os = "macos")]
mod imp {
    use objc2_foundation::NSBundle;
    use objc2_service_management::{SMAppService, SMAppServiceStatus};

    pub fn available() -> bool {
        let bundle = NSBundle::mainBundle();
        bundle.bundleIdentifier().is_some() && bundle.bundlePath().to_string().ends_with(".app")
    }

    pub fn is_enabled() -> bool {
        available() && unsafe { SMAppService::mainAppService().status() } == SMAppServiceStatus::Enabled
    }

    pub fn set_enabled(on: bool) -> Result<(), String> {
        let service = unsafe { SMAppService::mainAppService() };
        let result = if on {
            unsafe { service.registerAndReturnError() }
        } else {
            unsafe { service.unregisterAndReturnError() }
        };
        result.map_err(|e| e.localizedDescription().to_string())?;
        // macOS can accept the request but want the user's OK first.
        if on && unsafe { service.status() } == SMAppServiceStatus::RequiresApproval {
            unsafe { SMAppService::openSystemSettingsLoginItems() };
            return Err(format!("Allow {} in System Settings > General > Login Items.", crate::APP_NAME));
        }
        Ok(())
    }
}

/// Windows: a value under the current user's Run key, which Windows starts at sign-in.
#[cfg(windows)]
mod imp {
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows::Win32::System::Registry::{
        RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
    };

    const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    /// The value's name: the app's name (crate::APP_NAME), which the uninstaller also
    /// takes away.
    const NAME: PCWSTR = w!("Pirouette");
    /// Its name up to 0.3.x (crate::OLD_NAME).
    const OLD_NAME: PCWSTR = w!("Convert Wheel");

    /// A development build would register a path that disappears on the next build.
    pub fn available() -> bool {
        !cfg!(debug_assertions)
    }

    /// The command Windows should run at sign-in: this copy of the app, quoted.
    fn command() -> Result<String, String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        Ok(format!("\"{}\"", exe.display()))
    }

    /// The command under value `name`, if there is one.
    fn read(name: PCWSTR) -> Option<String> {
        let mut buf = [0u16; 1024];
        let mut size = (buf.len() * 2) as u32;
        let read = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                RUN,
                name,
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if read.is_err() {
            return None;
        }
        let len = (size as usize / 2).saturating_sub(1).min(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }

    /// On only if the entry starts this copy; an entry left by a moved or removed copy
    /// counts as off, and turning it on rewrites it.
    pub fn is_enabled() -> bool {
        read(NAME).is_some_and(|v| command().is_ok_and(|c| v.eq_ignore_ascii_case(&c)))
    }

    /// Open at Login under the old name (an MSI update from 0.3.x keeps it; the .exe
    /// update moves it itself, windows/hooks.nsh): the same choice under the new name,
    /// starting this copy, and the old value goes.
    pub fn move_old_entry() {
        if !available() || read(OLD_NAME).is_none() {
            return;
        }
        match set_enabled(true) {
            Ok(()) => {
                let _ = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, OLD_NAME) };
                crate::log::line("Open at Login now under the new name");
            }
            Err(e) => crate::log::line(&format!("couldn't move Open at Login to the new name: {e}")),
        }
    }

    #[cfg(test)]
    #[test]
    fn the_run_values_carry_the_app_names() {
        assert_eq!(unsafe { NAME.to_string() }.unwrap(), crate::APP_NAME);
        assert_eq!(unsafe { OLD_NAME.to_string() }.unwrap(), crate::OLD_NAME);
    }

    pub fn set_enabled(on: bool) -> Result<(), String> {
        let result = if on {
            let value: Vec<u16> = command()?.encode_utf16().chain([0]).collect();
            unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    RUN,
                    NAME,
                    REG_SZ.0,
                    Some(value.as_ptr().cast()),
                    (value.len() * 2) as u32,
                )
            }
        } else {
            match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, NAME) } {
                // Already off.
                e if e == ERROR_FILE_NOT_FOUND => return Ok(()),
                e => e,
            }
        };
        result.ok().map_err(|e| e.message())
    }
}

/// Linux: an XDG autostart entry, which GNOME, KDE and the others start at login.
#[cfg(target_os = "linux")]
mod imp {
    use std::fs;
    use std::path::PathBuf;

    use super::exec_quote;

    fn entry() -> Option<PathBuf> {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(config.join("autostart/convert-wheel.desktop"))
    }

    /// A development build would register a path that disappears on the next build.
    pub fn available() -> bool {
        !cfg!(debug_assertions) && entry().is_some()
    }

    pub fn is_enabled() -> bool {
        entry().is_some_and(|e| e.is_file())
    }

    pub fn set_enabled(on: bool) -> Result<(), String> {
        let entry = entry().ok_or("no home folder")?;
        if !on {
            return match fs::remove_file(&entry) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            };
        }
        // An AppImage runs from a temporary mount; $APPIMAGE is the file itself.
        let exe = std::env::var_os("APPIMAGE")
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .map_or_else(|| std::env::current_exe().map_err(|e| e.to_string()), Ok)?;
        let desktop = format!(
            "[Desktop Entry]\nType=Application\nName={}\nExec={}\nX-GNOME-Autostart-enabled=true\nNoDisplay=true\n",
            crate::APP_NAME,
            exec_quote(&exe.to_string_lossy())
        );
        fs::create_dir_all(entry.parent().ok_or("no autostart folder")?)
            .and_then(|_| fs::write(&entry, desktop))
            .map_err(|e| e.to_string())
    }
}

/// Quotes a path for a desktop file's Exec line, as the Desktop Entry spec asks:
/// double quotes, a backslash before `"` `` ` `` `$` `\`, and `%` doubled.
#[cfg(any(target_os = "linux", test))]
fn exec_quote(path: &str) -> String {
    let mut out = String::from("\"");
    for c in path.chars() {
        match c {
            '"' | '`' | '$' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '%' => out.push_str("%%"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn exec_lines_are_quoted_per_the_spec() {
        assert_eq!(super::exec_quote("/opt/a b/app"), "\"/opt/a b/app\"");
        assert_eq!(super::exec_quote("/x/100%/\"q\"/$HOME"), "\"/x/100%%/\\\"q\\\"/\\$HOME\"");
    }
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
mod imp {
    pub fn available() -> bool {
        false
    }
    pub fn is_enabled() -> bool {
        false
    }
    pub fn set_enabled(_on: bool) -> Result<(), String> {
        Err("not supported on this system yet".into())
    }
}

pub use imp::*;
