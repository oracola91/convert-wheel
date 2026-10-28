//! The Transparency slider in the menu's Wheel Glass (macOS), like the sliders in
//! Control Center. Tauri's menus have no sliders, so a native one goes into that
//! submenu, under its "Transparency" line, the first time the menu opens, and is set
//! to the saved value every time it opens. It is greyed out with No Liquid Glass.

use std::cell::RefCell;
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSMenu, NSMenuDidBeginTrackingNotification, NSMenuItem, NSSlider, NSView};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSPoint, NSRect, NSSize, NSString};
use tauri::{AppHandle, Manager};

use crate::settings::{Store, Transparency};

/// Marks the slider's menu item, so it goes in only once.
const TAG: isize = 0x5C1D;
/// The slider row in the menu. Points.
const WIDTH: f64 = 240.0;
const HEIGHT: f64 = 30.0;

pub struct Ivars {
    app: AppHandle,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ConvertWheelGlassSlider"]
    #[ivars = Ivars]
    struct Target;

    impl Target {
        /// The slider was let go: save where it is. The next wheel opens with it.
        #[unsafe(method(changed:))]
        fn changed(&self, sender: &NSSlider) {
            let value = sender.doubleValue().round().clamp(0.0, 100.0) as u8;
            let app = &self.ivars().app;
            if let Err(e) = app.state::<Store>().update(|s| s.transparency = Transparency(value)) {
                crate::notify(app, "Couldn't save the setting", &e);
            }
        }
    }
);

thread_local! {
    static TARGET: RefCell<Option<Retained<Target>>> = const { RefCell::new(None) };
    static SLIDER: RefCell<Option<Retained<NSSlider>>> = const { RefCell::new(None) };
}

/// Watches for the app's menu opening. Call once, on the main thread.
pub fn install(app: &AppHandle, mtm: MainThreadMarker) {
    let target = Target::alloc(mtm).set_ivars(Ivars { app: app.clone() });
    // SAFETY: NSObject's init, on an object allocated just now.
    let target: Retained<Target> = unsafe { msg_send![super(target), init] };
    TARGET.with(|t| *t.borrow_mut() = Some(target));
    let app = app.clone();
    let opened = RcBlock::new(move |note: NonNull<NSNotification>| {
        // SAFETY: AppKit passes a valid notification.
        let note = unsafe { note.as_ref() };
        if let Some(menu) = note.object().and_then(|o| o.downcast::<NSMenu>().ok()) {
            place(&app, &menu);
        }
    });
    // No queue: the slider is placed before the menu is drawn. The observer stays for
    // as long as the app runs, so its token is never needed.
    let token = unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(NSMenuDidBeginTrackingNotification),
            None,
            None,
            &opened,
        )
    };
    std::mem::forget(token);
}

/// Puts the slider into Wheel Glass if it isn't there yet, and sets it to what is saved.
fn place(app: &AppHandle, menu: &NSMenu) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(glass) = menu.itemWithTitle(&NSString::from_str("Wheel Glass")).and_then(|item| item.submenu()) else {
        return;
    };
    let slider = match glass.itemWithTag(TAG) {
        Some(_) => SLIDER.with(|s| s.borrow().clone()),
        None => Some(add_slider(&glass, mtm)),
    };
    if let Some(slider) = slider {
        let settings = app.state::<Store>().get();
        slider.setDoubleValue(f64::from(settings.transparency.0));
        slider.setEnabled(!settings.no_liquid_glass);
    }
}

fn add_slider(glass: &NSMenu, mtm: MainThreadMarker) -> Retained<NSSlider> {
    let row = NSView::initWithFrame(NSView::alloc(mtm), NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WIDTH, HEIGHT)));
    let frame = NSRect::new(NSPoint::new(20.0, 5.0), NSSize::new(WIDTH - 40.0, HEIGHT - 10.0));
    let slider = NSSlider::initWithFrame(NSSlider::alloc(mtm), frame);
    slider.setMinValue(0.0);
    slider.setMaxValue(100.0);
    // Saved once, when the knob is let go.
    slider.setContinuous(false);
    TARGET.with(|t| {
        if let Some(target) = &*t.borrow() {
            let target: &AnyObject = target;
            // SAFETY: the target lives as long as the app (TARGET) and has `changed:`.
            unsafe {
                slider.setTarget(Some(target));
                slider.setAction(Some(sel!(changed:)));
            }
        }
    });
    row.addSubview(&slider);
    let item = NSMenuItem::new(mtm);
    item.setTag(TAG);
    item.setView(Some(&row));
    glass.addItem(&item);
    SLIDER.with(|s| *s.borrow_mut() = Some(slider.clone()));
    slider
}
