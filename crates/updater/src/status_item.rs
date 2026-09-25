//! macOS menu-bar status item. The app crate forbids unsafe, so the
//! NSStatusItem lives here. Clicking it orders the window forward; the GUI
//! reads [`take_status_click`] on the next frame.

use objc2::{
    AnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained, sel,
};
use objc2_app_kit::{
    NSApplication, NSButton, NSCellImagePosition, NSControl, NSImage, NSImageScaling, NSStatusBar,
    NSStatusBarButton, NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::{NSData, NSObject, NSObjectProtocol, NSSize, ns_string};
use std::{
    cell::RefCell,
    sync::atomic::{AtomicBool, Ordering},
};

static CLICKED: AtomicBool = AtomicBool::new(false);

struct StatusIvars;

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = StatusIvars]
    struct StatusTarget;

    unsafe impl NSObjectProtocol for StatusTarget {}

    impl StatusTarget {
        #[unsafe(method(showTerminator:))]
        fn show_terminator(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            let app = NSApplication::sharedApplication(mtm);
            app.unhide(None);
            app.activate();
            for window in app.windows().iter() {
                if window.isMiniaturized() {
                    window.deminiaturize(None);
                }
                window.makeKeyAndOrderFront(None);
            }
            CLICKED.store(true, Ordering::Release);
        }
    }
);

struct Bar {
    item: Retained<NSStatusItem>,
    _target: Retained<StatusTarget>,
    png: Vec<u8>,
}

thread_local! {
    static BAR: RefCell<Option<Bar>> = const { RefCell::new(None) };
}

pub fn sync_status_item(png: &[u8], width_pt: f32, height_pt: f32) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    BAR.with(|slot| {
        let mut slot = slot.borrow_mut();
        let bar = slot.get_or_insert_with(|| install(mtm));
        if bar.png == png {
            return;
        }
        apply_image(&bar.item, mtm, png, width_pt, height_pt);
        bar.png = png.to_vec();
    });
}

pub fn take_status_click() -> bool {
    CLICKED.swap(false, Ordering::AcqRel)
}

fn install(mtm: MainThreadMarker) -> Bar {
    let target = new_target(mtm);
    let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
    item.setAutosaveName(Some(ns_string!("Terminator")));
    if let Some(button) = item.button(mtm) {
        let status: &NSStatusBarButton = &button;
        let control: &NSControl = status.as_ref();
        unsafe {
            control.setTarget(Some(&*target));
            control.setAction(Some(sel!(showTerminator:)));
        }
    }
    Bar {
        item,
        _target: target,
        png: Vec::new(),
    }
}

fn new_target(mtm: MainThreadMarker) -> Retained<StatusTarget> {
    unsafe { msg_send![super(StatusTarget::alloc(mtm).set_ivars(StatusIvars)), init] }
}

fn apply_image(
    item: &NSStatusItem,
    mtm: MainThreadMarker,
    png: &[u8],
    width_pt: f32,
    height_pt: f32,
) {
    let Some(button) = item.button(mtm) else {
        return;
    };
    let data = NSData::with_bytes(png);
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
        return;
    };
    image.setSize(NSSize::new(f64::from(width_pt), f64::from(height_pt)));
    image.setTemplate(false);
    let status: &NSStatusBarButton = &button;
    let button: &NSButton = status.as_ref();
    button.setImage(Some(&image));
    button.setImagePosition(NSCellImagePosition::ImageOnly);
    button.setImageScaling(NSImageScaling::ScaleProportionallyDown);
}
