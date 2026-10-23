//! The wheel's drop target on macOS.
//!
//! A transparent view on top of the web page takes the drag instead of the webview.
//! That lets it swap the dragged file's big icon and long name for a small icon, so
//! they stop covering the wheel, and work out which segment is under the cursor.

use std::cell::Cell;
use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, ProtocolObject};
use objc2::{define_class, msg_send, ClassType, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSDragOperation, NSDraggingInfo, NSDraggingItem,
    NSDraggingItemEnumerationOptions, NSHapticFeedbackManager, NSHapticFeedbackPattern,
    NSHapticFeedbackPerformanceTime, NSHapticFeedbackPerformer, NSPasteboardTypeFileURL, NSView,
    NSWindowOrderingMode, NSWorkspace,
};
use objc2_foundation::{NSArray, NSDictionary, NSInteger, NSPoint, NSRect, NSSize, NSURL};
use tauri::{AppHandle, Emitter, Manager};

use crate::wheel::{self, Hover, WheelState};
use crate::{trigger, wheel_window};

/// Size of the dragged icon while it is over the wheel.
const ICON: f64 = 28.0;

pub struct Ivars {
    app: AppHandle,
    hover: Cell<Hover>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "ConvertWheelDropView"]
    #[ivars = Ivars]
    pub struct DropView;

    impl DropView {
        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            self.shrink_drag_image(sender);
            self.track(sender)
        }

        #[unsafe(method(draggingUpdated:))]
        fn dragging_updated(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            self.track(sender)
        }

        #[unsafe(method(draggingExited:))]
        fn dragging_exited(&self, _sender: Option<&ProtocolObject<dyn NSDraggingInfo>>) {
            self.set_hover(Hover::NONE);
        }

        #[unsafe(method(prepareForDragOperation:))]
        fn prepare_for_drag_operation(&self, _sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            true
        }

        #[unsafe(method(performDragOperation:))]
        fn perform_drag_operation(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            let app = &self.ivars().app;
            let format = self
                .hover(sender)
                .segment
                .and_then(|i| app.state::<WheelState>().formats.lock().ok()?.get(i).copied());
            let paths = trigger::file_paths(&sender.draggingPasteboard());
            match format {
                Some(format) if !paths.is_empty() => {
                    trigger::hide_wheel(app, true);
                    crate::enqueue(app, paths, format.to_string());
                    true
                }
                _ => {
                    trigger::hide_wheel(app, false);
                    false
                }
            }
        }
    }
);

impl DropView {
    fn new(mtm: MainThreadMarker, frame: NSRect, app: AppHandle) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars { app, hover: Cell::new(Hover::NONE) });
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    fn hover(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> Hover {
        let p = self.convertPoint_fromView(sender.draggingLocation(), None);
        let size = self.bounds().size;
        let n = self.ivars().app.state::<WheelState>().formats.lock().map(|f| f.len()).unwrap_or(0);
        wheel::hover_at(p.x - size.width / 2.0, p.y - size.height / 2.0, n)
    }

    /// Lights up the segment under the cursor and accepts the drop only there. Anywhere
    /// else (the middle, the corners) the drop is refused, so macOS slides the file back.
    fn track(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
        let hover = self.hover(sender);
        self.set_hover(hover);
        if hover.segment.is_some() {
            NSDragOperation::Copy
        } else {
            NSDragOperation::None
        }
    }

    /// Tells the page what to light up, only when it changes. Moving onto a new format
    /// also gives a light tick on a Force Touch trackpad, at the same moment.
    fn set_hover(&self, hover: Hover) {
        let ivars = self.ivars();
        if ivars.hover.replace(hover) == hover {
            return;
        }
        let _ = ivars.app.emit("wheel-hover", hover);
        if hover.segment.is_some() {
            NSHapticFeedbackManager::defaultPerformer().performFeedbackPattern_performanceTime(
                NSHapticFeedbackPattern::Alignment,
                NSHapticFeedbackPerformanceTime::Now,
            );
        }
    }

    /// Replaces each dragged file's image with its small icon, just below and right of
    /// the cursor. Several files stack with a small offset and macOS adds a count badge.
    fn shrink_drag_image(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) {
        let cursor = self.convertPoint_fromView(sender.draggingLocation(), None);
        let workspace = NSWorkspace::sharedWorkspace();
        let count = Rc::new(Cell::new(0));
        let counted = Rc::clone(&count);
        let block = RcBlock::new(move |item: NonNull<NSDraggingItem>, index: NSInteger, _stop: NonNull<Bool>| {
            let item = unsafe { item.as_ref() };
            let Ok(url) = item.item().downcast::<NSURL>() else {
                return;
            };
            let Some(path) = url.path() else {
                return;
            };
            let icon = workspace.iconForFile(&path);
            icon.setSize(NSSize::new(ICON, ICON));
            let shift = index.min(3) as f64 * 3.0;
            let origin = NSPoint::new(cursor.x + 6.0 + shift, cursor.y - 6.0 - ICON - shift);
            let contents: &AnyObject = &icon;
            unsafe { item.setDraggingFrame_contents(NSRect::new(origin, NSSize::new(ICON, ICON)), Some(contents)) };
            counted.set(counted.get() + 1);
        });
        let classes = NSArray::from_slice(&[NSURL::class()]);
        unsafe {
            sender.enumerateDraggingItemsWithOptions_forView_classes_searchOptions_usingBlock(
                NSDraggingItemEnumerationOptions::ClearNonenumeratedImages,
                Some(self),
                &classes,
                &NSDictionary::new(),
                &block,
            );
        }
        if count.get() > 1 {
            sender.setNumberOfValidItemsForDrop(count.get());
        }
    }
}

/// Puts the drop view over the wheel window's web page. Call on the main thread.
pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let mtm = MainThreadMarker::new().expect("the drop view is installed on the main thread");
    let Some(ns_window) = wheel_window::ns_window(app) else {
        return Ok(());
    };
    let Some(content) = ns_window.contentView() else {
        return Ok(());
    };
    let view = DropView::new(mtm, content.bounds(), app.clone());
    view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable);
    view.registerForDraggedTypes(&NSArray::from_slice(&[unsafe { NSPasteboardTypeFileURL }]));
    content.addSubview_positioned_relativeTo(&view, NSWindowOrderingMode::Above, None);
    Ok(())
}
