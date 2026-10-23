//! Open at Login. macOS: the system's login items (System Settings > General >
//! Login Items); only an app in a bundle can be one, so the switch is off limits when
//! the app runs straight from Terminal. Windows: the Run key in the registry.

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

#[cfg(not(any(target_os = "macos", windows)))]
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
