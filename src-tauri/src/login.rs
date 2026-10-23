//! Open at Login, through the system's login items (System Settings > General >
//! Login Items). Only an app in a bundle can be a login item, so the switch is off
//! limits when the app runs straight from Terminal.

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

#[cfg(not(target_os = "macos"))]
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
