//! The wheel window's native parts on macOS: glass behind the ring (Liquid Glass on
//! macOS 26 and later, frosted before), the dragged file's picture in the hub (its
//! icon at once, then Quick Look's thumbnail when it comes), placement at the cursor,
//! and the fade in and out.
//! Everything here runs on the main thread.

use std::cell::RefCell;
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, Bool};
use objc2::{AllocAnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAnimatablePropertyContainer, NSAnimationContext, NSAutoresizingMaskOptions, NSBezierPath, NSColor,
    NSEvent, NSGlassEffectView, NSGlassEffectViewStyle, NSImage, NSImageScaling, NSImageView, NSScreen,
    NSVisualEffectBlendingMode,
    NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView, NSView, NSWindow, NSWindowOrderingMode,
    NSWorkspace,
};
use objc2_foundation::{NSError, NSPoint, NSRect, NSSize, NSString, NSURL};
use objc2_quick_look_thumbnailing::{
    QLThumbnailGenerationRequest, QLThumbnailGenerationRequestRepresentationTypes, QLThumbnailGenerator,
    QLThumbnailRepresentation,
};
use tauri::{AppHandle, Manager};

use crate::settings::{Glass, Store};
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
    /// The two glasses behind the ring, made once: the frosted material, and Liquid
    /// Glass where macOS has it. Each open shows the one chosen in the menu.
    static FROSTED: RefCell<Option<Retained<NSVisualEffectView>>> = const { RefCell::new(None) };
    static LIQUID: RefCell<Option<Retained<NSGlassEffectView>>> = const { RefCell::new(None) };
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

    install_glass(&content, mtm);
    apply_glass(app);

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

/// Puts both glasses under the web page, once: glass added after the window first
/// showed never appeared. With Reduce Transparency on, macOS draws each one solid.
fn install_glass(content: &NSView, mtm: MainThreadMarker) {
    let frosted = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), content.bounds());
    frosted.setMaterial(NSVisualEffectMaterial::Popover);
    frosted.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    // The app is never the active one, so the glass must not follow the active state.
    frosted.setState(NSVisualEffectState::Active);
    frosted.setMaskImage(Some(&disc(wheel::R_OUT)));
    frosted.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable);
    content.addSubview_positioned_relativeTo(&frosted, NSWindowOrderingMode::Below, None);
    FROSTED.with(|f| *f.borrow_mut() = Some(frosted));
    // Liquid Glass: macOS 26 and later.
    if AnyClass::get(c"NSGlassEffectView").is_some() {
        let (c, r) = (wheel::SIZE / 2.0, wheel::R_OUT);
        let frame = NSRect::new(NSPoint::new(c - r, c - r), NSSize::new(2.0 * r, 2.0 * r));
        let liquid = NSGlassEffectView::initWithFrame(NSGlassEffectView::alloc(mtm), frame);
        liquid.setCornerRadius(r);
        content.addSubview_positioned_relativeTo(&liquid, NSWindowOrderingMode::Below, None);
        LIQUID.with(|l| *l.borrow_mut() = Some(liquid));
    }
}

/// Shows the glass the menu's Wheel Glass asks for and hides the other: the frosted
/// material the wheel always had, or Liquid Glass, standard or clear. Without Liquid
/// Glass every choice is frosted.
fn apply_glass(app: &AppHandle) {
    let kind = app.state::<Store>().get().glass;
    let liquid = LIQUID.with(|l| l.borrow().clone());
    let use_liquid = liquid.is_some() && kind != Glass::Frosted;
    if let Some(liquid) = &liquid {
        let style = if kind == Glass::Clear { NSGlassEffectViewStyle::Clear } else { NSGlassEffectViewStyle::Regular };
        liquid.setStyle(style);
        liquid.setHidden(!use_liquid);
    }
    FROSTED.with(|f| {
        if let Some(frosted) = &*f.borrow() {
            frosted.setHidden(use_liquid);
        }
    });
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
    if let Some(path) = first_path {
        show_thumbnail(app, path);
    }
    apply_glass(app);
    place_at_cursor(&window, mtm);
    window.setAlphaValue(0.0);
    window.orderFrontRegardless();
    window.invalidateShadow();
    fade(window, 1.0, OPEN_SECONDS, || {});
}

/// An image handed from Quick Look's queue to the main thread, which alone touches it.
struct Thumbnail(Retained<NSImage>);
// SAFETY: the image is made on Quick Look's queue and only used on the main thread
// after that; nothing uses it on two threads at once.
unsafe impl Send for Thumbnail {}

/// Asks Quick Look for a picture of the file (a photo, a video's frame, a document's
/// first page) and puts it in the hub in place of the icon when it comes, unless the
/// wheel closed or another opened in the meantime. No picture: the icon stays.
fn show_thumbnail(app: &AppHandle, path: &str) {
    let generation = app.state::<WheelState>().generation();
    let url = NSURL::fileURLWithPath(&NSString::from_str(path));
    let request = unsafe {
        QLThumbnailGenerationRequest::initWithFileAtURL_size_scale_representationTypes(
            QLThumbnailGenerationRequest::alloc(),
            &url,
            NSSize::new(ICON, ICON),
            2.0,
            QLThumbnailGenerationRequestRepresentationTypes::Thumbnail,
        )
    };
    let app = app.clone();
    let done = RcBlock::new(move |representation: *mut QLThumbnailRepresentation, _error: *mut NSError| {
        // SAFETY: Quick Look passes a valid representation or nil.
        let Some(representation) = (unsafe { representation.as_ref() }) else {
            return;
        };
        let image = Thumbnail(unsafe { representation.NSImage() });
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            let image = image;
            let state = handle.state::<WheelState>();
            if !state.is_open() || !state.is_current(generation) {
                return;
            }
            HUB_ICON.with(|h| {
                if let Some(view) = &*h.borrow() {
                    view.setImage(Some(&image.0));
                }
            });
        });
    });
    unsafe { QLThumbnailGenerator::sharedGenerator().generateBestRepresentationForRequest_completionHandler(&request, &done) };
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
