//! macOS Services: "Open in Pirouette Wheel" in Finder's right-click menu (under
//! Services or Quick Actions) and in the app menus' Services. Finder hands over the
//! selected files itself, so the app never reads another app's selection and asks for
//! no permission (council 2026-11-10, keyboard mode E). A key for it can be set in
//! System Settings > Keyboard > Keyboard Shortcuts > Services. The entry is declared
//! in Info.plist (NSServices); its port name must match PORT.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::NSPasteboard;
use objc2_foundation::{NSObject, NSString};
use tauri::AppHandle;

use crate::trigger;

/// Info.plist's NSPortName for the entry.
const PORT: &str = "Pirouette";

pub struct Ivars {
    app: AppHandle,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PirouetteServices"]
    #[ivars = Ivars]
    pub struct Services;

    impl Services {
        /// Info.plist's NSMessage "openInWheel": the wheel opens on the files, for
        /// the keyboard.
        #[unsafe(method(openInWheel:userData:error:))]
        fn open_in_wheel(&self, pasteboard: &NSPasteboard, _user_data: Option<&NSString>, _error: *mut *mut NSString) {
            let paths = trigger::file_paths(pasteboard);
            trigger::open_for_keys(&self.ivars().app, paths);
        }
    }
);

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    fn NSRegisterServicesProvider(provider: &AnyObject, name: &NSString);
    fn NSUpdateDynamicServices();
}

thread_local! {
    /// Kept for as long as the app runs: AppKit holds the provider only weakly.
    static PROVIDER: RefCell<Option<Retained<Services>>> = const { RefCell::new(None) };
}

/// Registers the provider under PORT and asks macOS to read the Services again, so a
/// freshly installed or renamed app shows the entry without logging out.
pub fn install(app: &AppHandle, mtm: MainThreadMarker) {
    let this = Services::alloc(mtm).set_ivars(Ivars { app: app.clone() });
    let provider: Retained<Services> = unsafe { msg_send![super(this), init] };
    // SAFETY: a live Objective-C object and a string, on the main thread, as AppKit
    // expects; the object is kept alive in PROVIDER.
    unsafe {
        NSRegisterServicesProvider(&provider, &NSString::from_str(PORT));
        NSUpdateDynamicServices();
    }
    PROVIDER.with(|p| *p.borrow_mut() = Some(provider));
}
