//! The shelf's window on macOS: black and grown out of the notch, or a glass box under
//! the menu bar icon. It takes dropped files and lets them be dragged out again.
//! Everything here runs on the main thread; the trigger thread calls in through the
//! `*_soon` functions.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::ptr::NonNull;
use std::thread;
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAnimatablePropertyContainer, NSAnimationContext, NSAutoresizingMaskOptions, NSBackingStoreType, NSBezierPath,
    NSColor, NSDragOperation, NSDraggingContext, NSDraggingInfo, NSDraggingItem, NSDraggingSession, NSDraggingSource,
    NSEvent, NSFont, NSHapticFeedbackManager, NSHapticFeedbackPattern, NSHapticFeedbackPerformanceTime,
    NSHapticFeedbackPerformer, NSImage, NSImageResizingMode, NSImageScaling, NSImageView, NSLineBreakMode, NSPanel,
    NSPasteboardTypeFileURL, NSScreen, NSStatusWindowLevel, NSTextAlignment, NSTextField, NSTrackingArea,
    NSTrackingAreaOptions, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState,
    NSVisualEffectView, NSWindowCollectionBehavior, NSWindowOcclusionState, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{NSArray, NSEdgeInsets, NSPoint, NSRect, NSSize, NSString, NSURL};
use tauri::{AppHandle, Manager};

use crate::shelf::{self, Anchor, Area, Look, Shelf};
use crate::{log, reveal, settings, trigger};

/// Resize and fade time. With Reduce Motion on, changes are instant.
const SECONDS: f64 = 0.2;
/// File icons in the open shelf, and the small one beside the notch or in the pill.
const ICON: f64 = 44.0;
const SMALL_ICON: f64 = 18.0;
/// How far the pointer moves with the button down before it is a drag, not a click.
const DRAG_START: f64 = 4.0;
/// How long the open shelf waits after the pointer left before it closes.
const CLOSE_DELAY: Duration = Duration::from_millis(250);
/// The clear button, at the body's top right.
const CLEAR: f64 = 18.0;
/// Corners of the glass box under the menu bar.
const GLASS_RADIUS: f64 = 14.0;

/// The panel and its parts, made once at start.
#[derive(Clone)]
struct Ui {
    panel: Retained<NSPanel>,
    glass: Retained<NSVisualEffectView>,
    view: Retained<ShelfView>,
    /// Up to three file icons; the last one is drawn in front.
    icons: [Retained<NSImageView>; 3],
    count: Retained<NSTextField>,
    title: Retained<NSTextField>,
    detail: Retained<NSTextField>,
    clear: Retained<NSImageView>,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
    static LOOK: Cell<Look> = const { Cell::new(Look::Hidden) };
    /// The frame of the screen the shelf is on.
    static HOME: Cell<Option<Area>> = const { Cell::new(None) };
    /// What the view draws: black around the notch, or nothing over the glass; and the
    /// height of the strip above the body (the notch's, or none).
    static NOTCH: Cell<bool> = const { Cell::new(false) };
    static TOP: Cell<f64> = const { Cell::new(0.0) };
    /// Where the clear button is while it shows, in the view.
    static CLEAR_AT: Cell<Option<NSRect>> = const { Cell::new(None) };
    /// Bumped at each change of look, so a late close can't undo a newer change.
    static TURN: Cell<u64> = const { Cell::new(0) };
}

pub struct Ivars {
    app: AppHandle,
    /// Where the button went down, until the pointer moves far enough for a drag.
    press: Cell<Option<NSPoint>>,
    /// The files on their way out, while they are being dragged out of the shelf.
    out: RefCell<Option<Vec<String>>>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "ConvertWheelShelfView"]
    #[ivars = Ivars]
    pub struct ShelfView;

    unsafe impl NSObjectProtocol for ShelfView {}

    impl ShelfView {
        /// Lays out from the top, where the shelf hangs from.
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        /// The app is never the active one, so the first click has to count, or
        /// nothing could be dragged out.
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        /// Clicks and drags anywhere on the shelf come here, not to its icons and labels.
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> *mut NSView {
            if inside(self.frame(), point) {
                (self as *const Self).cast_mut().cast()
            } else {
                std::ptr::null_mut()
            }
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            draw(self.bounds());
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            self.ivars().press.set(Some(self.local(event)));
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            let Some(start) = self.ivars().press.get() else {
                return;
            };
            let p = self.local(event);
            if (p.x - start.x).hypot(p.y - start.y) >= DRAG_START {
                self.ivars().press.set(None);
                self.drag_out(event, p);
            }
        }

        /// A click: on the clear button empties the shelf, on the open shelf shows the
        /// files in Finder, on the small one opens it.
        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            if self.ivars().press.take().is_none() {
                return;
            }
            let app = &self.ivars().app;
            let p = self.local(event);
            if CLEAR_AT.get().is_some_and(|r| inside(grow(r, 6.0), p)) {
                app.state::<Shelf>().clear();
                log::line("shelf cleared");
                show(app, Look::Hidden);
                return;
            }
            match LOOK.get() {
                Look::Open => {
                    let held: Vec<PathBuf> = app.state::<Shelf>().held().into_iter().map(PathBuf::from).collect();
                    reveal::show(app, held);
                }
                Look::Compact => show(app, Look::Open),
                _ => {}
            }
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: &NSEvent) {
            if LOOK.get() == Look::Compact {
                show(&self.ivars().app, Look::Open);
            }
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            if LOOK.get() == Look::Open && self.ivars().out.borrow().is_none() {
                close_later(&self.ivars().app);
            }
        }

        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            self.offer(sender)
        }

        #[unsafe(method(draggingUpdated:))]
        fn dragging_updated(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            self.offer(sender)
        }

        #[unsafe(method(draggingExited:))]
        fn dragging_exited(&self, _sender: Option<&ProtocolObject<dyn NSDraggingInfo>>) {
            if LOOK.get() == (Look::Drop { over: true }) {
                show(&self.ivars().app, Look::Drop { over: false });
            }
        }

        #[unsafe(method(prepareForDragOperation:))]
        fn prepare_for_drag_operation(&self, _sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            true
        }

        #[unsafe(method(performDragOperation:))]
        fn perform_drag_operation(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            self.hold(sender)
        }
    }

    unsafe impl NSDraggingSource for ShelfView {
        /// Out of the app the files may be moved or copied; Finder picks as it does for
        /// its own drags (move on the same disk, copy to another, Option for a copy).
        /// Onto the shelf itself: nothing.
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn source_operations(&self, _session: &NSDraggingSession, context: NSDraggingContext) -> NSDragOperation {
            if context == NSDraggingContext::OutsideApplication {
                NSDragOperation::Copy | NSDragOperation::Move | NSDragOperation::Generic
            } else {
                NSDragOperation::None
            }
        }

        /// Files that landed somewhere leave the shelf. The shelf never deletes or moves
        /// anything itself: whoever took the drop did that.
        #[unsafe(method(draggingSession:endedAtPoint:operation:))]
        fn session_ended(&self, _session: &NSDraggingSession, _at: NSPoint, operation: NSDragOperation) {
            let out = self.ivars().out.take().unwrap_or_default();
            let app = &self.ivars().app;
            if operation != NSDragOperation::None {
                app.state::<Shelf>().remove(&out);
                let how = if operation.contains(NSDragOperation::Move) { "moved" } else { "copied or used" };
                log::line(&format!("shelf: {} file(s) dragged out ({how})", out.len()));
            }
            show(app, resting(app));
        }
    }
);

impl ShelfView {
    fn new(mtm: MainThreadMarker, frame: NSRect, app: AppHandle) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars { app, press: Cell::new(None), out: RefCell::new(None) });
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    fn local(&self, event: &NSEvent) -> NSPoint {
        self.convertPoint_fromView(event.locationInWindow(), None)
    }

    /// The drag came from the shelf itself.
    fn own_drag(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
        let me = (self as *const Self).cast::<AnyObject>();
        sender.draggingSource().is_some_and(|source| std::ptr::eq(Retained::as_ptr(&source), me))
    }

    /// A drag over the shelf: lit up and taken as a copy, unless it is the shelf's own.
    fn offer(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
        if self.own_drag(sender) {
            return NSDragOperation::None;
        }
        if LOOK.get() != (Look::Drop { over: true }) {
            show(&self.ivars().app, Look::Drop { over: true });
        }
        NSDragOperation::Copy
    }

    /// Holds the dropped files. Only their paths are kept: the files stay where they
    /// are, and the drag's source is told it was a copy, so it never removes them.
    fn hold(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
        if self.own_drag(sender) {
            return false;
        }
        let paths = trigger::file_paths(&sender.draggingPasteboard());
        if paths.is_empty() {
            return false;
        }
        let app = &self.ivars().app;
        let shelf = app.state::<Shelf>();
        let added = shelf.add(paths);
        log::line(&format!("shelf: holding {added} more, {} in all", shelf.held().len()));
        NSHapticFeedbackManager::defaultPerformer()
            .performFeedbackPattern_performanceTime(NSHapticFeedbackPattern::Generic, NSHapticFeedbackPerformanceTime::Now);
        show(app, Look::Compact);
        true
    }

    /// Starts dragging every held file that is still there, from where the pointer is.
    fn drag_out(&self, event: &NSEvent, at: NSPoint) {
        let app = &self.ivars().app;
        let shelf = app.state::<Shelf>();
        shelf.prune();
        let held = shelf.held();
        if held.is_empty() {
            show(app, Look::Hidden);
            return;
        }
        let workspace = NSWorkspace::sharedWorkspace();
        let items: Vec<Retained<NSDraggingItem>> = held
            .iter()
            .enumerate()
            .map(|(i, path)| {
                let path = NSString::from_str(path);
                let url = NSURL::fileURLWithPath(&path);
                let item = NSDraggingItem::initWithPasteboardWriter(NSDraggingItem::alloc(), ProtocolObject::from_ref(&*url));
                let icon = workspace.iconForFile(&path);
                let shift = i.min(3) as f64 * 3.0;
                let frame = NSRect::new(NSPoint::new(at.x - ICON / 2.0 + shift, at.y - ICON / 2.0 + shift), NSSize::new(ICON, ICON));
                let contents: &AnyObject = &icon;
                unsafe { item.setDraggingFrame_contents(frame, Some(contents)) };
                item
            })
            .collect();
        self.ivars().out.replace(Some(held));
        let session = self.beginDraggingSessionWithItems_event_source(
            &NSArray::from_retained_slice(&items),
            event,
            ProtocolObject::from_ref(self),
        );
        session.setAnimatesToStartingPositionsOnCancelOrFail(true);
    }
}

/// Makes the panel. It stays hidden until there is something to show.
pub fn install(app: &AppHandle, mtm: MainThreadMarker) {
    let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(shelf::OPEN_WIDTH, shelf::BODY));
    // A panel that never makes the app active, above the menu bar so it can cover the
    // strip around the notch, on every Space and over full screen apps too.
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    let panel: Retained<NSPanel> = unsafe {
        msg_send![NSPanel::alloc(mtm), initWithContentRect: rect, styleMask: style, backing: NSBackingStoreType::Buffered, defer: true]
    };
    // SAFETY: the panel is kept for as long as the app runs and only ever ordered out.
    unsafe { panel.setReleasedWhenClosed(false) };
    panel.setLevel(NSStatusWindowLevel + 1);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setHasShadow(true);
    panel.setHidesOnDeactivate(false);
    panel.setBecomesKeyOnlyIfNeeded(true);

    let content = NSView::initWithFrame(NSView::alloc(mtm), rect);
    let sizable = NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable;
    let glass = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), rect);
    glass.setMaterial(NSVisualEffectMaterial::Popover);
    glass.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    glass.setState(NSVisualEffectState::Active);
    glass.setMaskImage(Some(&rounded(GLASS_RADIUS)));
    glass.setAutoresizingMask(sizable);
    content.addSubview(&glass);

    let view = ShelfView::new(mtm, rect, app.clone());
    view.setAutoresizingMask(sizable);
    view.registerForDraggedTypes(&NSArray::from_slice(&[unsafe { NSPasteboardTypeFileURL }]));
    let owner: &AnyObject = &view;
    let tracking = unsafe {
        NSTrackingArea::initWithRect_options_owner_userInfo(
            NSTrackingArea::alloc(),
            NSRect::ZERO,
            NSTrackingAreaOptions::MouseEnteredAndExited | NSTrackingAreaOptions::ActiveAlways | NSTrackingAreaOptions::InVisibleRect,
            Some(owner),
            None,
        )
    };
    view.addTrackingArea(&tracking);
    content.addSubview(&view);

    let image = || {
        let v = NSImageView::initWithFrame(NSImageView::alloc(mtm), NSRect::ZERO);
        v.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
        v.setEditable(false);
        view.addSubview(&v);
        v
    };
    let icons = [image(), image(), image()];
    let clear = image();
    clear.setImage(symbol("xmark.circle.fill", "Clear the shelf").as_deref());
    let label = |size: f64, bold: bool| {
        let l = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        let font = if bold { NSFont::boldSystemFontOfSize(size) } else { NSFont::systemFontOfSize(size) };
        l.setFont(Some(&font));
        l.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
        view.addSubview(&l);
        l
    };
    let count = label(12.0, true);
    count.setAlignment(NSTextAlignment::Center);
    let title = label(13.0, true);
    let detail = label(11.0, false);

    panel.setContentView(Some(&content));
    UI.with(|u| *u.borrow_mut() = Some(Ui { panel, glass, view, icons, count, title, detail, clear }));
    // Files held before the last quit: show that they are still there.
    show(app, resting(app));
}

/// The trigger saw a file drag under way: opens the shelf for a drop when the drag
/// comes near it, and closes it again when the drag moves away.
pub fn dragging_soon(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || dragging(&handle));
}

/// The file drag ended, dropped or not.
pub fn drag_ended_soon(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if matches!(LOOK.get(), Look::Drop { .. }) {
            show(&handle, resting(&handle));
        }
    });
}

/// Keeps a showing shelf in place after screens were added, removed or rearranged.
pub fn refresh_soon(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let look = LOOK.get();
        if !matches!(look, Look::Compact | Look::Open) {
            return;
        }
        let (Some(mtm), Some(ui)) = (MainThreadMarker::new(), UI.with(|u| u.borrow().clone())) else {
            return;
        };
        let moved = home_anchor(mtm, &handle).and_then(|a| a.frame(look)).map(rect) != Some(ui.panel.frame());
        if moved {
            show(&handle, look);
        }
    });
}

/// The shelf was switched on or off in the menu.
pub fn setting_changed(app: &AppHandle) {
    show(app, resting(app));
}

fn dragging(app: &AppHandle) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let out = UI.with(|u| u.borrow().as_ref().is_some_and(|ui| ui.view.ivars().out.borrow().is_some()));
    if off(app) || out {
        return;
    }
    let p = NSEvent::mouseLocation();
    let Some(screen) = NSScreen::screens(mtm).iter().find(|s| area(s.frame()).contains(p.x, p.y)) else {
        return;
    };
    let near = anchor_on(app, &screen).zone().contains(p.x, p.y);
    let here = HOME.get() == Some(area(screen.frame()));
    match LOOK.get() {
        Look::Drop { .. } if near && here => {}
        _ if near => {
            HOME.set(Some(area(screen.frame())));
            show(app, Look::Drop { over: false });
        }
        Look::Drop { .. } => show(app, resting(app)),
        _ => {}
    }
}

fn off(app: &AppHandle) -> bool {
    app.state::<settings::Store>().get().shelf_off
}

/// How the shelf shows when nothing is going on: small while it holds files.
fn resting(app: &AppHandle) -> Look {
    if app.state::<Shelf>().is_empty() {
        Look::Hidden
    } else {
        Look::Compact
    }
}

/// Changes how the shelf shows, moving and resizing it with a short animation.
fn show(app: &AppHandle, look: Look) {
    let (Some(mtm), Some(ui)) = (MainThreadMarker::new(), UI.with(|u| u.borrow().clone())) else {
        return;
    };
    let shelf = app.state::<Shelf>();
    let gone = if look == Look::Open { shelf.prune() } else { 0 };
    let held = shelf.held();
    let mut look = if off(app) { Look::Hidden } else { look };
    // An empty shelf only shows to take a drop.
    if held.is_empty() && matches!(look, Look::Compact | Look::Open) {
        look = Look::Hidden;
    }
    let was = LOOK.replace(look);
    TURN.set(TURN.get() + 1);
    let anchor = home_anchor(mtm, app);
    let Some(frame) = anchor.and_then(|a| a.frame(look)) else {
        CLEAR_AT.set(None);
        fade_out(ui.panel);
        return;
    };
    let notch = anchor.is_some_and(|a| a.is_notch());
    NOTCH.set(notch);
    TOP.set(match anchor {
        Some(Anchor::Notch(n)) => n.h,
        _ => 0.0,
    });
    ui.glass.setHidden(notch);
    let frame = rect(frame);
    layout(&ui, look, notch, frame.size, &held, gone);
    ui.view.setNeedsDisplay(true);
    let seconds = if reduce_motion() { 0.0 } else { SECONDS };
    let panel = ui.panel.clone();
    if was == Look::Hidden || !panel.isVisible() {
        panel.setFrame_display(frame, true);
        panel.setAlphaValue(0.0);
        panel.orderFrontRegardless();
        animate(seconds, move || panel.animator().setAlphaValue(1.0), || {});
    } else {
        animate(
            seconds,
            move || {
                panel.animator().setFrame_display(frame, true);
                panel.animator().setAlphaValue(1.0);
            },
            || {},
        );
    }
    ui.panel.invalidateShadow();
}

/// Fades the panel out, then takes it off the screen unless it changed again meanwhile.
fn fade_out(panel: Retained<NSPanel>) {
    if !panel.isVisible() {
        return;
    }
    let turn = TURN.get();
    let seconds = if reduce_motion() { 0.0 } else { SECONDS };
    let (fade, hide) = (panel.clone(), panel);
    animate(
        seconds,
        move || fade.animator().setAlphaValue(0.0),
        move || {
            if TURN.get() == turn {
                hide.orderOut(None);
            }
        },
    );
}

/// The open shelf closes once the pointer has stayed away from it for a moment.
fn close_later(app: &AppHandle) {
    let turn = TURN.get();
    let app = app.clone();
    thread::spawn(move || {
        thread::sleep(CLOSE_DELAY);
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            let away = UI.with(|u| u.borrow().as_ref().is_some_and(|ui| !inside(ui.panel.frame(), NSEvent::mouseLocation())));
            if TURN.get() == turn && LOOK.get() == Look::Open && away {
                show(&handle, resting(&handle));
            }
        });
    });
}

/// Puts the icons and words where `look` wants them, in a panel of `size`.
fn layout(ui: &Ui, look: Look, notch: bool, size: NSSize, held: &[String], gone: usize) {
    let (w, h) = (size.width, size.height);
    let top = TOP.get();
    let ink = if notch { NSColor::whiteColor() } else { NSColor::labelColor() };
    let soft = if notch { NSColor::colorWithWhite_alpha(1.0, 0.6) } else { NSColor::secondaryLabelColor() };
    for v in ui.icons.iter().chain([&ui.clear]) {
        v.setHidden(true);
    }
    for l in [&ui.count, &ui.title, &ui.detail] {
        l.setHidden(true);
    }
    CLEAR_AT.set(None);
    let workspace = NSWorkspace::sharedWorkspace();
    let icon_of = |p: &String| workspace.iconForFile(&NSString::from_str(p));
    let put = |v: &NSView, x: f64, y: f64, w: f64, h: f64| {
        v.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(w, h)));
        v.setHidden(false);
    };
    let say = |l: &NSTextField, text: &str, color: &NSColor| {
        l.setStringValue(&NSString::from_str(text));
        l.setTextColor(Some(color));
    };
    match look {
        Look::Hidden => {}
        Look::Compact => {
            // The newest file's icon on the left, how many on the right.
            let (icon_x, count_x, count_w) =
                if notch { ((shelf::WING - SMALL_ICON) / 2.0 + 2.0, w - shelf::WING, shelf::WING - 4.0) } else { (10.0, 32.0, w - 38.0) };
            let icon = &ui.icons[2];
            icon.setImage(held.last().map(icon_of).as_deref());
            icon.setContentTintColor(None);
            put(icon, icon_x, (h - SMALL_ICON) / 2.0, SMALL_ICON, SMALL_ICON);
            say(&ui.count, &shelf::count(held.len()), &ink);
            put(&ui.count, count_x, (h - 16.0) / 2.0, count_w, 16.0);
        }
        Look::Open | Look::Drop { .. } => {
            let mid = top + (h - top) / 2.0;
            let text_x = 22.0 + ICON + 14.0;
            let text_w = w - text_x - 22.0 - CLEAR;
            if let Look::Drop { .. } = look {
                let icon = &ui.icons[2];
                icon.setImage(symbol("tray.and.arrow.down", "Shelf").as_deref());
                icon.setContentTintColor(Some(&soft));
                put(icon, 22.0 + 6.0, mid - (ICON - 12.0) / 2.0, ICON - 12.0, ICON - 12.0);
                let title = if held.is_empty() { "Drop to hold here" } else { "Drop to add to the shelf" };
                say(&ui.title, title, &ink);
                say(&ui.detail, "Nothing moves until you drag it out", &soft);
            } else {
                // Up to three icons fanned out, the newest in front.
                for (k, path) in held.iter().rev().take(3).enumerate() {
                    let icon = &ui.icons[2 - k];
                    icon.setImage(Some(&icon_of(path)));
                    icon.setContentTintColor(None);
                    put(icon, 22.0 + 5.0 * k as f64, mid - ICON / 2.0 - 4.0 * k as f64, ICON, ICON);
                }
                say(&ui.title, &shelf::title(held), &ink);
                let detail = match gone {
                    0 => "Drag out to move. Option copies.".to_string(),
                    n => format!("{n} no longer there. Drag out to move."),
                };
                say(&ui.detail, &detail, &soft);
                let at = NSRect::new(NSPoint::new(w - 12.0 - CLEAR, top + 10.0), NSSize::new(CLEAR, CLEAR));
                ui.clear.setContentTintColor(Some(&soft));
                put(&ui.clear, at.origin.x, at.origin.y, CLEAR, CLEAR);
                CLEAR_AT.set(Some(at));
            }
            put(&ui.title, text_x, mid - 19.0, text_w, 18.0);
            put(&ui.detail, text_x, mid + 2.0, text_w, 16.0);
        }
    }
}

/// The view's own drawing: the black shape around the notch, and the dashed outline
/// that shows where to drop.
fn draw(b: NSRect) {
    let look = LOOK.get();
    let top = TOP.get();
    if NOTCH.get() {
        let r = if look == Look::Compact { 10.0 } else { 20.0 };
        NSColor::blackColor().set();
        NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(b, r, r).fill();
        // Square at the top: it hangs from the edge of the screen like the notch.
        NSBezierPath::fillRect(NSRect::new(b.origin, NSSize::new(b.size.width, r)));
    }
    if let Look::Drop { over } = look {
        let body = NSRect::new(
            NSPoint::new(b.origin.x + 10.0, b.origin.y + top + 8.0),
            NSSize::new(b.size.width - 20.0, b.size.height - top - 18.0),
        );
        let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(body, 12.0, 12.0);
        let accent = NSColor::controlAccentColor();
        if over {
            accent.colorWithAlphaComponent(0.2).set();
            path.fill();
        }
        let line = if over {
            accent
        } else if NOTCH.get() {
            NSColor::colorWithWhite_alpha(1.0, 0.45)
        } else {
            NSColor::secondaryLabelColor()
        };
        line.set();
        path.setLineWidth(1.5);
        let dash: [f64; 2] = [6.0, 4.0];
        unsafe { path.setLineDash_count_phase(dash.as_ptr(), 2, 0.0) };
        path.stroke();
    }
}

/// The shelf's screen (where files were last dropped or dragged near it), or the main
/// screen when that one is gone.
fn home_anchor(mtm: MainThreadMarker, app: &AppHandle) -> Option<Anchor> {
    let screens = NSScreen::screens(mtm);
    let screen = screens
        .iter()
        .find(|s| HOME.get() == Some(area(s.frame())))
        .or_else(|| NSScreen::mainScreen(mtm))
        .or_else(|| screens.firstObject())?;
    HOME.set(Some(area(screen.frame())));
    Some(anchor_on(app, &screen))
}

fn anchor_on(app: &AppHandle, screen: &NSScreen) -> Anchor {
    let frame = area(screen.frame());
    let top = screen.safeAreaInsets().top;
    // The notch is what the two free areas beside the camera leave between them.
    let notch = (top > 0.0).then(|| {
        let (left, right) = (screen.auxiliaryTopLeftArea().size.width, screen.auxiliaryTopRightArea().size.width);
        Area { x: frame.x + left, y: frame.max_y() - top, w: frame.w - left - right, h: top }
    });
    Anchor::on(frame, area(screen.visibleFrame()), notch, icon_frame(app))
}

/// The menu bar icon, while it can be seen.
fn icon_frame(app: &AppHandle) -> Option<Area> {
    let tray = app.tray_by_id("main")?;
    tray.with_inner_tray_icon(|inner| {
        let mtm = MainThreadMarker::new()?;
        let window = inner.ns_status_item()?.button(mtm)?.window()?;
        let seen = window.isVisible() && window.occlusionState().contains(NSWindowOcclusionState::Visible);
        seen.then(|| area(window.frame()))
    })
    .ok()
    .flatten()
}

fn reduce_motion() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

fn symbol(name: &str, description: &str) -> Option<Retained<NSImage>> {
    NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), Some(&NSString::from_str(description)))
}

/// A black rounded rectangle that stretches to any size, for the glass's mask.
fn rounded(r: f64) -> Retained<NSImage> {
    let side = 2.0 * r + 1.0;
    let draw = RcBlock::new(move |rect: NSRect| -> Bool {
        NSColor::blackColor().set();
        NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, r, r).fill();
        Bool::YES
    });
    let image = NSImage::imageWithSize_flipped_drawingHandler(NSSize::new(side, side), false, &draw);
    image.setCapInsets(NSEdgeInsets { top: r, left: r, bottom: r, right: r });
    image.setResizingMode(NSImageResizingMode::Stretch);
    image
}

fn animate(seconds: f64, changes: impl Fn() + 'static, done: impl Fn() + 'static) {
    let changes = RcBlock::new(move |ctx: NonNull<NSAnimationContext>| {
        unsafe { ctx.as_ref() }.setDuration(seconds);
        changes();
    });
    let done = RcBlock::new(done);
    NSAnimationContext::runAnimationGroup_completionHandler(&changes, Some(&done));
}

fn area(r: NSRect) -> Area {
    Area { x: r.origin.x, y: r.origin.y, w: r.size.width, h: r.size.height }
}

fn rect(a: Area) -> NSRect {
    NSRect::new(NSPoint::new(a.x, a.y), NSSize::new(a.w, a.h))
}

fn inside(r: NSRect, p: NSPoint) -> bool {
    area(r).contains(p.x, p.y)
}

fn grow(r: NSRect, by: f64) -> NSRect {
    NSRect::new(NSPoint::new(r.origin.x - by, r.origin.y - by), NSSize::new(r.size.width + 2.0 * by, r.size.height + 2.0 * by))
}
