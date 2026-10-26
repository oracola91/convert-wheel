//! Notifications on macOS, through UserNotifications.
//!
//! The notification plugin uses the older NSUserNotification API on macOS, and a click
//! on one of its notifications never reached the app. Here a click shows the files the
//! notification reported, also from Notification Center later on. The paths travel
//! inside the notification, so nothing has to remember which notification was which.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AnyThread, DefinedClass};
use objc2_foundation::{NSArray, NSDictionary, NSError, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationContent,
    UNNotificationDefaultActionIdentifier, UNNotificationPresentationOptions, UNNotificationRequest,
    UNNotificationResponse, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::{log, reveal};

/// Where a notification keeps the paths of the files it reported.
const FILES: &str = "files";

pub struct Ivars {
    app: AppHandle,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "ConvertWheelNotificationDelegate"]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        /// Shows the banner even while the wheel has the focus.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            handler: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            handler.call((UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::List,));
        }

        /// A click on the notification itself, not on a button, shows its files.
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            handler: &block2::DynBlock<dyn Fn()>,
        ) {
            // SAFETY: a constant string that UserNotifications defines.
            let click = unsafe { UNNotificationDefaultActionIdentifier };
            if response.actionIdentifier().isEqualToString(click) {
                let files = files_in(&response.notification().request().content());
                if !files.is_empty() {
                    reveal::show(&self.ivars().app, files);
                }
            }
            handler.call(());
        }
    }
);

/// Becomes the delegate of the app's notifications. Call it once, before any
/// notification is shown, so a click on one never goes unheard.
pub fn install(app: &AppHandle) {
    let delegate = Delegate::alloc().set_ivars(Ivars { app: app.clone() });
    // SAFETY: NSObject's init, on an object allocated just now.
    let delegate: Retained<Delegate> = unsafe { msg_send![super(delegate), init] };
    UNUserNotificationCenter::currentNotificationCenter()
        .setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    // The center holds its delegate weakly, and the app needs it until it quits.
    std::mem::forget(delegate);
}

/// Shows a notification; a click on it shows `files`. The first one asks the user for
/// permission. If macOS refuses this app outright (rather than the user saying no), the
/// notification plugin shows it instead, without the click.
pub fn show(app: &AppHandle, title: &str, body: &str, files: &[PathBuf]) {
    static IDS: AtomicU32 = AtomicU32::new(1);
    static TOLD: AtomicBool = AtomicBool::new(false);

    let identifier = format!("{}-{}", std::process::id(), IDS.fetch_add(1, Ordering::SeqCst));
    let content = content(title, body, files);
    let request =
        UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(&identifier), &content, None);
    let (app, title, body) = (app.clone(), title.to_owned(), body.to_owned());
    let asked = RcBlock::new(move |granted: Bool, error: *mut NSError| {
        // SAFETY: UserNotifications passes a valid error or none.
        if let Some(error) = unsafe { error.as_ref() } {
            if !TOLD.swap(true, Ordering::SeqCst) {
                log::line(&format!("macOS refused notifications ({}), using the older kind", error.localizedDescription()));
            }
            if let Err(e) = app.notification().builder().title(&title).body(&body).show() {
                log::line(&format!("notification not shown ({e}): {title}: {body}"));
            }
        } else if !granted.as_bool() {
            if !TOLD.swap(true, Ordering::SeqCst) {
                log::line("notifications are off in System Settings");
            }
        } else {
            let added = RcBlock::new(|error: *mut NSError| {
                // SAFETY: as above.
                if let Some(error) = unsafe { error.as_ref() } {
                    log::line(&format!("notification not shown ({})", error.localizedDescription()));
                }
            });
            UNUserNotificationCenter::currentNotificationCenter()
                .addNotificationRequest_withCompletionHandler(&request, Some(&added));
        }
    });
    // Asks only the first time; after that it answers at once with the user's choice.
    UNUserNotificationCenter::currentNotificationCenter()
        .requestAuthorizationWithOptions_completionHandler(UNAuthorizationOptions::Alert, &asked);
}

/// A notification's title, text and the paths of its files.
fn content(title: &str, body: &str, files: &[PathBuf]) -> Retained<UNMutableNotificationContent> {
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    let paths: Vec<Retained<NSString>> = files.iter().filter_map(|f| f.to_str()).map(NSString::from_str).collect();
    if !paths.is_empty() {
        let info = NSDictionary::from_slices(&[&*NSString::from_str(FILES)], &[&*NSArray::from_retained_slice(&paths)]);
        let info: &AnyObject = &info;
        if let Some(info) = info.downcast_ref::<NSDictionary>() {
            // SAFETY: a string key and an array of strings, which a notification can store.
            unsafe { content.setUserInfo(info) };
        }
    }
    content
}

/// The paths a notification reported; none for one that reported no files.
fn files_in(content: &UNNotificationContent) -> Vec<PathBuf> {
    let Some(list) = content.userInfo().objectForKey(&NSString::from_str(FILES)) else {
        return Vec::new();
    };
    let Ok(list) = list.downcast::<NSArray>() else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|path| path.downcast::<NSString>().ok())
        .map(|path| PathBuf::from(path.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_files_travel_inside_the_notification() {
        let files = vec![
            PathBuf::from("/Users/a/Downloads/Holiday, \"best\" 2.jpg"),
            PathBuf::from("/Users/a/Desktop/Ünïcödé\nline.pdf"),
        ];
        assert_eq!(files_in(&content("Converted to JPG", "2 files", &files)), files);
        assert!(files_in(&content("Convert Wheel settings", "Problem", &[])).is_empty());
    }
}
