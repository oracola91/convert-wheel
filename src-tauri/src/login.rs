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
        result.map_err(|e| e.localizedDescription().to_string())
    }
}

/// Windows: a value under the current user's Run key, which Windows starts at sign-in.
#[cfg(windows)]
mod imp {
    use windows::core::{w, PCWSTR};
    use windows::Win32::System::Registry::{
        RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
    };

    const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    const NAME: PCWSTR = w!("Convert Wheel");

    /// A development build would register a path that disappears on the next build.
    pub fn available() -> bool {
        !cfg!(debug_assertions)
    }

    pub fn is_enabled() -> bool {
        unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN, NAME, RRF_RT_REG_SZ, None, None, None) }.is_ok()
    }

    pub fn set_enabled(on: bool) -> Result<(), String> {
        let result = if on {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let value: Vec<u16> = format!("\"{}\"", exe.display()).encode_utf16().chain([0]).collect();
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
            unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, NAME) }
        };
        result.ok().map_err(|e| e.message())
    }
}

/// Linux: an XDG autostart entry, which GNOME, KDE and the others start at login.
#[cfg(target_os = "linux")]
mod imp {
    use std::fs;
    use std::path::PathBuf;

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
            .map_or_else(|| std::env::current_exe().map_err(|e| e.to_string()), Ok)?;
        let desktop = format!(
            "[Desktop Entry]\nType=Application\nName=Convert Wheel\nExec=\"{}\"\nX-GNOME-Autostart-enabled=true\nNoDisplay=true\n",
            exe.display()
        );
        fs::create_dir_all(entry.parent().ok_or("no autostart folder")?)
            .and_then(|_| fs::write(&entry, desktop))
            .map_err(|e| e.to_string())
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
