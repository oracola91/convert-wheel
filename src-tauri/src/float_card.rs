//! Float on macOS: the dragged files on a small card that floats above every window,
//! to drag out again later into any app or folder. Each Float makes a card of its own,
//! next to the pointer. Its icon and name are where the files are taken out; the rest
//! of the card moves the card. Its close button puts it away, and so does a drag that
//! moved the files somewhere else. Everything here runs on the main thread.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::thread;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSCursor, NSDragOperation, NSDraggingContext, NSDraggingItem, NSDraggingSession, NSDraggingSource,
    NSEvent, NSFloatingWindowLevel, NSFont, NSImageScaling, NSImageView, NSLineBreakMode, NSPanel, NSScreen, NSTextField,
    NSTrackingArea, NSTrackingAreaOptions, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
    NSWindowCollectionBehavior, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString, NSURL};
use tauri::AppHandle;

use crate::log;
use crate::shelf_window::{rounded, symbol};

const WIDTH: f64 = 250.0;
const HEIGHT: f64 = 64.0;
const ICON: f64 = 40.0;
/// The close button, at the top right, and the move sign under it.
const CLOSE: f64 = 16.0;
const MOVE_SIGN: f64 = 13.0;
const RADIUS: f64 = 12.0;
/// How far the pointer moves with the button down before it is a drag, not a click.
const DRAG_START: f64 = 4.0;
/// Each new card sits a little lower and to the right of the one before.
const CASCADE: f64 = 22.0;

thread_local! {
    /// The cards on screen, kept until they are put away.
    static CARDS: RefCell<Vec<Retained<NSPanel>>> = const { RefCell::new(Vec::new()) };
}

pub struct Ivars {
    app: AppHandle,
    paths: Vec<String>,
    /// Where the button went down, until the pointer moves far enough for a drag.
    press: Cell<Option<NSPoint>>,
    /// The icon and the words on it: a drag from there takes the files out.
    grab: Cell<NSRect>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "PirouetteFloatCard"]
    #[ivars = Ivars]
    pub struct CardView;

    unsafe impl NSObjectProtocol for CardView {}

    impl CardView {
        /// Laid out from the top.
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        /// The app is never the active one, so the first click has to count.
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        /// Clicks and drags anywhere on the card come here, not to its icon and words.
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> *mut NSView {
            if contains(self.frame(), point) {
                (self as *const Self).cast_mut().cast()
            } else {
                std::ptr::null_mut()
            }
        }

        /// On the icon or the name: the start of taking the files out. On the close
        /// button: a click. Anywhere else: the card follows the pointer to a new place.
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let p = self.local(event);
            if contains(self.ivars().grab.get(), p) || contains(grow(close_at(), 6.0), p) {
                self.ivars().press.set(Some(p));
            } else if let Some(window) = self.window() {
                NSCursor::closedHandCursor().set();
                window.performWindowDragWithEvent(event);
            }
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            let Some(start) = self.ivars().press.get() else {
                return;
            };
            let p = self.local(event);
            if (p.x - start.x).hypot(p.y - start.y) >= DRAG_START && contains(self.ivars().grab.get(), start) {
                self.ivars().press.set(None);
                self.drag_out(event, p);
            }
        }

        /// An open hand over the part that moves the card, the arrow elsewhere.
        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, event: &NSEvent) {
            self.show_cursor(self.local(event));
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            self.show_cursor(self.local(event));
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            NSCursor::arrowCursor().set();
        }

        /// A click on the close button puts the card away.
        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            if self.ivars().press.take().is_some() && contains(grow(close_at(), 6.0), self.local(event)) {
                self.put_away();
            }
        }
    }

    unsafe impl NSDraggingSource for CardView {
        /// Out of the app the files may be moved or copied, as Finder picks for its own
        /// drags (move on the same disk, copy to another, Option for a copy). Within the
        /// app, onto the wheel (Shift while dragging) or the shelf: a copy, which leaves
        /// the files where they are for the conversion or the shelf to use.
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn source_operations(&self, _session: &NSDraggingSession, context: NSDraggingContext) -> NSDragOperation {
            if context == NSDraggingContext::OutsideApplication {
                NSDragOperation::Copy | NSDragOperation::Move | NSDragOperation::Generic
            } else {
                NSDragOperation::Copy
            }
        }

        /// Files moved elsewhere (or gone) take the card with them; after a copy it
        /// stays, for the next place they are needed.
        #[unsafe(method(draggingSession:endedAtPoint:operation:))]
        fn session_ended(&self, _session: &NSDraggingSession, _at: NSPoint, operation: NSDragOperation) {
            if operation != NSDragOperation::None {
                log::line(&format!("float: {} file(s) dragged out", self.ivars().paths.len()));
            }
            if !self.ivars().paths.iter().any(|p| Path::new(p).exists()) {
                self.put_away();
            }
        }
    }
);

impl CardView {
    fn new(mtm: MainThreadMarker, frame: NSRect, app: AppHandle, paths: Vec<String>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars { app, paths, press: Cell::new(None), grab: Cell::new(NSRect::ZERO) });
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    fn local(&self, event: &NSEvent) -> NSPoint {
        self.convertPoint_fromView(event.locationInWindow(), None)
    }

    fn show_cursor(&self, p: NSPoint) {
        let moves = !contains(self.ivars().grab.get(), p) && !contains(grow(close_at(), 6.0), p);
        if moves { NSCursor::openHandCursor() } else { NSCursor::arrowCursor() }.set();
    }

    /// Starts dragging the card's files that are still there, from the pointer.
    fn drag_out(&self, event: &NSEvent, at: NSPoint) {
        let workspace = NSWorkspace::sharedWorkspace();
        let items: Vec<Retained<NSDraggingItem>> = self
            .ivars()
            .paths
            .iter()
            .filter(|p| Path::new(p).exists())
            .enumerate()
            .map(|(i, path)| {
                let path = NSString::from_str(path);
                let url = NSURL::fileURLWithPath(&path);
                let item = NSDraggingItem::initWithPasteboardWriter(NSDraggingItem::alloc(), ProtocolObject::from_ref(&*url));
                let shift = i.min(3) as f64 * 3.0;
                let frame = NSRect::new(NSPoint::new(at.x - ICON / 2.0 + shift, at.y - ICON / 2.0 + shift), NSSize::new(ICON, ICON));
                let icon = workspace.iconForFile(&path);
                let contents: &AnyObject = &icon;
                unsafe { item.setDraggingFrame_contents(frame, Some(contents)) };
                item
            })
            .collect();
        if items.is_empty() {
            self.put_away();
            return;
        }
        let session = self.beginDraggingSessionWithItems_event_source(
            &NSArray::from_retained_slice(&items),
            event,
            ProtocolObject::from_ref(self),
        );
        session.setAnimatesToStartingPositionsOnCancelOrFail(true);
    }

    /// Hides the card now and lets it go on the next turn of the main loop, not while
    /// its own view is still busy with the click or the drag.
    fn put_away(&self) {
        let Some(window) = self.window() else {
            return;
        };
        window.orderOut(None);
        let gone = Retained::as_ptr(&window).cast::<AnyObject>() as usize;
        let app = self.ivars().app.clone();
        thread::spawn(move || {
            let _ = app.run_on_main_thread(move || {
                CARDS.with(|cards| cards.borrow_mut().retain(|c| Retained::as_ptr(c).cast::<AnyObject>() as usize != gone));
            });
        });
    }
}

/// Floats the files on a new card next to the pointer.
pub fn open(app: &AppHandle, files: Vec<PathBuf>) {
    let paths: Vec<String> = files.iter().filter(|f| f.exists()).filter_map(|f| f.to_str().map(String::from)).collect();
    if paths.is_empty() {
        return;
    }
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(mtm) = MainThreadMarker::new() {
            make(&handle, mtm, paths);
        }
    });
}

fn make(app: &AppHandle, mtm: MainThreadMarker, paths: Vec<String>) {
    let cards = CARDS.with(|c| c.borrow().len());
    let frame = place(mtm, cards);
    // A panel that never makes the app active, above other windows, on every Space.
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    let panel: Retained<NSPanel> = unsafe {
        msg_send![NSPanel::alloc(mtm), initWithContentRect: frame, styleMask: style, backing: NSBackingStoreType::Buffered, defer: false]
    };
    // SAFETY: CARDS keeps the panel until it is put away; closing never frees it.
    unsafe { panel.setReleasedWhenClosed(false) };
    panel.setLevel(NSFloatingWindowLevel);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setHasShadow(true);
    panel.setHidesOnDeactivate(false);
    panel.setBecomesKeyOnlyIfNeeded(true);

    let bounds = NSRect::new(NSPoint::new(0.0, 0.0), frame.size);
    let content = NSView::initWithFrame(NSView::alloc(mtm), bounds);
    let glass = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), bounds);
    glass.setMaterial(NSVisualEffectMaterial::Popover);
    glass.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    glass.setState(NSVisualEffectState::Active);
    glass.setMaskImage(Some(&rounded(RADIUS)));
    content.addSubview(&glass);

    let view = CardView::new(mtm, bounds, app.clone(), paths.clone());
    let image = |frame: NSRect| {
        let v = NSImageView::initWithFrame(NSImageView::alloc(mtm), frame);
        v.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
        v.setEditable(false);
        view.addSubview(&v);
        v
    };
    let icon = image(NSRect::new(NSPoint::new(12.0, (HEIGHT - ICON) / 2.0), NSSize::new(ICON, ICON)));
    icon.setImage(Some(&NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(&paths[0]))));
    image(close_at()).setImage(symbol("xmark.circle.fill", "Put the card away").as_deref());
    // The sign for the part that moves the card: the right side, under the close button.
    let sign = image(NSRect::new(NSPoint::new(WIDTH - 8.0 - (CLOSE + MOVE_SIGN) / 2.0, HEIGHT - 10.0 - MOVE_SIGN), NSSize::new(MOVE_SIGN, MOVE_SIGN)));
    sign.setImage(symbol("arrow.up.and.down.and.arrow.left.and.right", "Drag here to move the card").as_deref());
    sign.setContentTintColor(Some(&NSColor::tertiaryLabelColor()));

    let (title, detail) = describe(&paths);
    let left = 12.0 + ICON + 10.0;
    let width = WIDTH - left - CLOSE - 16.0;
    let label = |text: &str, top: f64, size: f64, bold: bool| {
        let l = NSTextField::labelWithString(&NSString::from_str(text), mtm);
        let font = if bold { NSFont::boldSystemFontOfSize(size) } else { NSFont::systemFontOfSize(size) };
        l.setFont(Some(&font));
        l.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
        l.setFrame(NSRect::new(NSPoint::new(left, top), NSSize::new(width, size + 5.0)));
        view.addSubview(&l);
        l
    };
    let name = label(&title, 13.0, 13.0, true);
    let note = label(&detail, 33.0, 11.0, false);
    note.setTextColor(Some(&NSColor::secondaryLabelColor()));
    // The handle for the files: the icon and as far as the words reach.
    let words = name.intrinsicContentSize().width.max(note.intrinsicContentSize().width).min(width);
    let top = (HEIGHT - ICON) / 2.0;
    view.ivars().grab.set(NSRect::new(NSPoint::new(8.0, top), NSSize::new(left - 8.0 + words + 4.0, ICON)));
    let owner: &AnyObject = &view;
    let tracking = unsafe {
        NSTrackingArea::initWithRect_options_owner_userInfo(
            NSTrackingArea::alloc(),
            NSRect::ZERO,
            NSTrackingAreaOptions::MouseEnteredAndExited
                | NSTrackingAreaOptions::MouseMoved
                | NSTrackingAreaOptions::ActiveAlways
                | NSTrackingAreaOptions::InVisibleRect,
            Some(owner),
            None,
        )
    };
    view.addTrackingArea(&tracking);
    content.addSubview(&view);

    panel.setContentView(Some(&content));
    panel.orderFrontRegardless();
    CARDS.with(|c| c.borrow_mut().push(panel));
    log::line(&format!("float: {} file(s) on a card", paths.len()));
}

/// The card's two lines: the file's name, or how many files there are.
fn describe(paths: &[String]) -> (String, String) {
    match paths {
        [one] => {
            let name = Path::new(one).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| one.clone());
            (name, "Drag it out when you need it".to_string())
        }
        many => (format!("{} files", many.len()), "Drag them out when you need them".to_string()),
    }
}

/// Where a new card goes: below and right of the pointer, after the cards already
/// there, inside the screen the pointer is on.
fn place(mtm: MainThreadMarker, cards: usize) -> NSRect {
    let at = NSEvent::mouseLocation();
    let shift = (cards % 6) as f64 * CASCADE;
    let (mut x, mut y) = (at.x + 18.0 + shift, at.y - HEIGHT - 18.0 - shift);
    let screen = NSScreen::screens(mtm).iter().find(|s| contains(s.frame(), at)).or_else(|| NSScreen::mainScreen(mtm));
    if let Some(screen) = screen {
        let v = screen.visibleFrame();
        x = x.min(v.origin.x + v.size.width - WIDTH - 8.0).max(v.origin.x + 8.0);
        y = y.min(v.origin.y + v.size.height - HEIGHT - 8.0).max(v.origin.y + 8.0);
    }
    NSRect::new(NSPoint::new(x, y), NSSize::new(WIDTH, HEIGHT))
}

fn close_at() -> NSRect {
    NSRect::new(NSPoint::new(WIDTH - CLOSE - 8.0, 8.0), NSSize::new(CLOSE, CLOSE))
}

fn contains(r: NSRect, p: NSPoint) -> bool {
    p.x >= r.origin.x && p.x <= r.origin.x + r.size.width && p.y >= r.origin.y && p.y <= r.origin.y + r.size.height
}

fn grow(r: NSRect, by: f64) -> NSRect {
    NSRect::new(NSPoint::new(r.origin.x - by, r.origin.y - by), NSSize::new(r.size.width + 2.0 * by, r.size.height + 2.0 * by))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_names_its_file_or_counts_them() {
        assert_eq!(describe(&["/tmp/a b/photo.jpg".into()]).0, "photo.jpg");
        assert_eq!(describe(&["/tmp/a".into(), "/tmp/b".into()]).0, "2 files");
        let close = close_at();
        assert!(close.origin.x + close.size.width < WIDTH && contains(grow(close, 6.0), NSPoint::new(WIDTH - 10.0, 10.0)));
    }
}
