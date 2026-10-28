//! The wheel window's native parts on macOS: glass behind the ring (Liquid Glass on
//! macOS 26 and later, frosted before), the dragged file's icon in the hub, placement
//! at the cursor, and the fade in and out.
//! Everything here runs on the main thread.

use std::cell::RefCell;
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, Bool};
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAnimatablePropertyContainer, NSAnimationContext, NSAutoresizingMaskOptions, NSBezierPath, NSColor,
    NSEvent, NSGlassEffectView, NSGlassEffectViewStyle, NSImage, NSImageScaling, NSImageView, NSScreen,
    NSVisualEffectBlendingMode,
    NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView, NSWindow, NSWindowOrderingMode,
    NSWorkspace,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use tauri::{AppHandle, Manager};

use crate::wheel::{self, WheelState};

/// The hub icon's size, and how far its centre sits above the wheel's centre so the
/// label fits underneath. Points.
const ICON: f64 = 52.0;
const ICON_LIFT: f64 = 12.0;
/// Fade times, matched by the scale animations in src/wheel.css.
const OPEN_SECONDS: f64 = 0.12;
const CLOSE_SECONDS: f64 = 0.14;

thread_local! {
    static HUB_ICON: RefCell<Option<Retained<NSImageView>>> = const { RefCell::new(None) };
}

pub fn ns_window(app: &AppHandle) -> Option<Retained<NSWindow>> {
    let win = app.get_webview_window("wheel")?;
    let ptr = win.ns_window().ok()?.cast::<NSWindow>();
    unsafe { Retained::retain(ptr) }
}

/// Adds the glass disc under the web page and the hub icon above it.
pub fn install(app: &AppHandle, mtm: MainThreadMarker) -> Result<(), String> {
    let window = ns_window(app).ok_or("the wheel window is missing")?;
    let content = window.contentView().ok_or("the wheel window has no content")?;
    let bounds = content.bounds();

    // Glass, as a disc. Liquid Glass where macOS has it (26 and later), the regular
    // kind: it always blurs what is behind, so the milky segments read the same on any
    // background. (The clear kind adapts to what is behind it: over a light window it
    // went almost invisible on one open and frosted on the next.) Before 26 the frosted
    // material. With Reduce Transparency on, macOS draws either one solid by itself.
    if AnyClass::get(c"NSGlassEffectView").is_some() {
        let (c, r) = (wheel::SIZE / 2.0, wheel::R_OUT);
        let frame = NSRect::new(NSPoint::new(c - r, c - r), NSSize::new(2.0 * r, 2.0 * r));
        let glass = NSGlassEffectView::initWithFrame(NSGlassEffectView::alloc(mtm), frame);
        glass.setStyle(NSGlassEffectViewStyle::Regular);
        glass.setCornerRadius(r);
        content.addSubview_positioned_relativeTo(&glass, NSWindowOrderingMode::Below, None);
    } else {
        let glass = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), bounds);
        glass.setMaterial(NSVisualEffectMaterial::Popover);
        glass.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        // The app is never the active one, so the glass must not follow the active state.
        glass.setState(NSVisualEffectState::Active);
        glass.setMaskImage(Some(&disc(wheel::R_OUT)));
        glass.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable);
        content.addSubview_positioned_relativeTo(&glass, NSWindowOrderingMode::Below, None);
    }

    let c = wheel::SIZE / 2.0;
    let frame = NSRect::new(NSPoint::new(c - ICON / 2.0, c - ICON / 2.0 + ICON_LIFT), NSSize::new(ICON, ICON));
    let icon = NSImageView::initWithFrame(NSImageView::alloc(mtm), frame);
    icon.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
    icon.setEditable(false);
    content.addSubview_positioned_relativeTo(&icon, NSWindowOrderingMode::Above, None);
    HUB_ICON.with(|h| *h.borrow_mut() = Some(icon));

    // A transparent window's shadow follows what is drawn, so this gives the disc a soft edge.
    window.setHasShadow(true);
    Ok(())
}

/// A black disc of radius `r` centred in the window, used as the glass mask.
fn disc(r: f64) -> Retained<NSImage> {
    let size = NSSize::new(wheel::SIZE, wheel::SIZE);
    let draw = RcBlock::new(move |_rect: NSRect| -> Bool {
        let c = wheel::SIZE / 2.0;
        let rect = NSRect::new(NSPoint::new(c - r, c - r), NSSize::new(2.0 * r, 2.0 * r));
        NSColor::blackColor().set();
        NSBezierPath::bezierPathWithOvalInRect(rect).fill();
        Bool::YES
    });
    NSImage::imageWithSize_flipped_drawingHandler(size, false, &draw)
}

/// Shows the wheel at the cursor with the first dragged file's icon in the hub.
pub fn open(app: &AppHandle, first_path: Option<&str>) {
    let (Some(mtm), Some(window)) = (MainThreadMarker::new(), ns_window(app)) else {
        return;
    };
    HUB_ICON.with(|h| {
        if let Some(view) = &*h.borrow() {
            let image = first_path.map(|p| NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(p)));
            view.setImage(image.as_deref());
        }
    });
    place_at_cursor(&window, mtm);
    window.setAlphaValue(0.0);
    window.orderFrontRegardless();
    window.invalidateShadow();
    fade(window, 1.0, OPEN_SECONDS, || {});
}

/// Fades the wheel out, then hides it unless a newer wheel opened in the meantime.
pub fn close(app: &AppHandle, generation: u64) {
    // A newer wheel may have opened before this ran; don't fade that one out.
    if !app.state::<WheelState>().is_current(generation) {
        return;
    }
    let Some(window) = ns_window(app) else {
        return;
    };
    let (app, hide) = (app.clone(), window.clone());
    fade(window, 0.0, CLOSE_SECONDS, move || {
        if app.state::<WheelState>().is_current(generation) {
            hide.orderOut(None);
        }
    });
}

/// Centres the wheel on the cursor, nudged inside the screen's usable area so no
/// part of it hides under the menu bar, the Dock or a screen edge.
fn place_at_cursor(window: &NSWindow, mtm: MainThreadMarker) {
    let p = NSEvent::mouseLocation();
    let s = wheel::SIZE;
    let (mut x, mut y) = (p.x - s / 2.0, p.y - s / 2.0);
    let screens = NSScreen::screens(mtm);
    let under_cursor = screens.iter().find(|screen| {
        let f = screen.frame();
        p.x >= f.origin.x && p.x <= f.origin.x + f.size.width && p.y >= f.origin.y && p.y <= f.origin.y + f.size.height
    });
    if let Some(screen) = under_cursor {
        let v = screen.visibleFrame();
        x = x.min(v.origin.x + v.size.width - s).max(v.origin.x);
        y = y.min(v.origin.y + v.size.height - s).max(v.origin.y);
    }
    window.setFrameOrigin(NSPoint::new(x, y));
}

fn fade(window: Retained<NSWindow>, to: f64, seconds: f64, done: impl Fn() + 'static) {
    let changes = RcBlock::new(move |ctx: NonNull<NSAnimationContext>| {
        unsafe { ctx.as_ref() }.setDuration(seconds);
        window.animator().setAlphaValue(to);
    });
    let done = RcBlock::new(done);
    NSAnimationContext::runAnimationGroup_completionHandler(&changes, Some(&done));
}
