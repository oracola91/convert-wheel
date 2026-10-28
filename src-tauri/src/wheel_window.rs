//! The wheel window's native parts on macOS: the frosted material behind the ring, as
//! strong as the menu's Transparency asks and frosted behind each word, the dragged
//! file's icon in the hub, placement at the cursor,
//! and the fade in and out.
//! Everything here runs on the main thread.

use std::cell::RefCell;
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::{AllocAnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAnimatablePropertyContainer, NSAnimationContext, NSAutoresizingMaskOptions, NSBitmapImageRep,
    NSDeviceRGBColorSpace, NSEvent, NSImage, NSImageScaling, NSImageView, NSScreen,
    NSVisualEffectBlendingMode,
    NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView, NSView, NSWindow, NSWindowOrderingMode,
    NSWorkspace,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use tauri::{AppHandle, Manager};

use crate::settings::Store;
use crate::wheel::{self, WheelState, WordBox};

/// The hub icon's size, and how far its centre sits above the wheel's centre so the
/// label fits underneath. Points.
const ICON: f64 = 52.0;
const ICON_LIFT: f64 = 12.0;
/// Fade times, matched by the scale animations in src/wheel.css.
const OPEN_SECONDS: f64 = 0.12;
const CLOSE_SECONDS: f64 = 0.14;
/// The glass mask has two pixels to a point, so its edges stay smooth on Retina screens.
const MASK_SCALE: f64 = 2.0;

thread_local! {
    static HUB_ICON: RefCell<Option<Retained<NSImageView>>> = const { RefCell::new(None) };
    /// The frosted material behind the ring, made once, shown when the menu says Frosted.
    static FROSTED: RefCell<Option<Retained<NSVisualEffectView>>> = const { RefCell::new(None) };
    /// The words on the wheel now, as its page measured them; empty while it is hidden.
    static WORDS: RefCell<Vec<WordBox>> = const { RefCell::new(Vec::new()) };
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

/// Puts the frosted material under the web page, once (glass added after the window
/// first showed never appeared). With Reduce Transparency on, macOS draws it solid.
fn install_glass(content: &NSView, mtm: MainThreadMarker) {
    let frosted = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), content.bounds());
    frosted.setMaterial(NSVisualEffectMaterial::Popover);
    frosted.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    // The app is never the active one, so the glass must not follow the active state.
    frosted.setState(NSVisualEffectState::Active);
    frosted.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable);
    content.addSubview_positioned_relativeTo(&frosted, NSWindowOrderingMode::Below, None);
    FROSTED.with(|f| *f.borrow_mut() = Some(frosted));
}

/// Wheel Glass in the menu: with Liquid Glass, the slider says how much of the frosted
/// material shows behind the wheel (all of it at frosted, none at clear), and behind
/// each word it shows almost fully, so the word reads over anything. It is drawn at
/// once, the same every open. No Liquid Glass: none, the page draws a solid disc.
fn apply_glass(app: &AppHandle) {
    let settings = app.state::<Store>().get();
    let blur = if settings.no_liquid_glass { 0.0 } else { settings.transparency.blur() };
    let words = if settings.no_liquid_glass { Vec::new() } else { WORDS.with(|w| w.borrow().clone()) };
    FROSTED.with(|f| {
        if let Some(view) = &*f.borrow() {
            if let Some(image) = mask(blur, &words) {
                view.setMaskImage(Some(&image));
            }
            view.setHidden(blur == 0.0 && words.is_empty());
        }
    });
}

/// The page drew a wheel and measured its words: frost the glass behind them.
pub fn set_words(app: &AppHandle, words: Vec<WordBox>) {
    WORDS.with(|w| *w.borrow_mut() = words);
    apply_glass(app);
}

/// The glass mask: its alpha is how strongly the frosted material shows at each point.
fn mask(blur: f64, words: &[WordBox]) -> Option<Retained<NSImage>> {
    let side = (wheel::SIZE * MASK_SCALE) as usize;
    // SAFETY: no planes are passed, so the rep makes its own buffer for the sizes given.
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            side as isize,
            side as isize,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            (side * 4) as isize,
            32,
        )
    }?;
    let data = rep.bitmapData();
    if data.is_null() {
        return None;
    }
    // SAFETY: the rep's own buffer, side rows of side * 4 bytes, used by nothing else yet.
    let pixels = unsafe { std::slice::from_raw_parts_mut(data, side * side * 4) };
    // Rows run from the top, like the page's coordinates. Black at the alpha: the
    // buffer is premultiplied, so the colour stays 0.
    for (i, pixel) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = (((i % side) as f64 + 0.5) / MASK_SCALE, ((i / side) as f64 + 0.5) / MASK_SCALE);
        let alpha = wheel::glass_alpha(x, y, blur, words);
        *pixel = [0, 0, 0, (alpha * 255.0).round() as u8];
    }
    let size = NSSize::new(wheel::SIZE, wheel::SIZE);
    rep.setSize(size);
    let image = NSImage::initWithSize(NSImage::alloc(), size);
    image.addRepresentation(&rep);
    Some(image)
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
    apply_glass(app);
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
            // The next wheel's page sends its own words; none of these may show on it.
            WORDS.with(|w| w.borrow_mut().clear());
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
