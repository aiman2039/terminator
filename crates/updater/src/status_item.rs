//! macOS menu-bar status item. The app crate forbids unsafe, so the
//! NSStatusItem lives here. Clicking it opens a menu of pending agent
//! notices plus Show Terminator. The GUI syncs the menu with
//! [`sync_status_menu`] and reads [`take_status_click`] /
//! [`take_status_selection`] on the next frame.

use super::StatusMenuItem;
use objc2::{
    AnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained, sel,
};
use objc2_app_kit::{
    NSApplication, NSButton, NSCellImagePosition, NSControl, NSImage, NSImageScaling, NSMenu,
    NSMenuItem, NSNormalWindowLevel, NSPopUpMenuWindowLevel, NSStatusBar, NSStatusBarButton,
    NSStatusItem, NSVariableStatusItemLength, NSWindowLevel,
};
use objc2_foundation::{NSData, NSObject, NSObjectProtocol, NSSize, NSString, ns_string};
use std::{
    cell::RefCell,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

static CLICKED: AtomicBool = AtomicBool::new(false);
static SELECTION: Mutex<Option<String>> = Mutex::new(None);

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
            order_front();
            CLICKED.store(true, Ordering::Release);
        }

        #[unsafe(method(selectNotice:))]
        fn select_notice(&self, sender: Option<&NSMenuItem>) {
            if let Some(tag) = sender.map(|item| item.tag())
                && let Ok(index) = usize::try_from(tag)
            {
                BAR.with(|slot| {
                    if let Some(id) = slot
                        .borrow()
                        .as_ref()
                        .and_then(|bar| bar.menu_ids.get(index))
                        .cloned()
                    {
                        *SELECTION.lock().unwrap() = Some(id);
                    }
                });
            }
            order_front();
        }
    }
);

fn order_front() {
    // Menu actions run inside menu tracking. Ordering every NSWindow front
    // from here includes the menu window, and AppKit then fades that window
    // to alpha 0 without ordering it out. It stays at popup level and
    // swallows clicks. Wait until tracking ends, drop any leftover menu
    // window, and raise only the document window.
    dispatch2::DispatchQueue::main().exec_async(|| {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        raise_document_window(mtm);
    });
}

fn raise_document_window(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    app.unhide(None);
    app.activate();
    for window in app.windows().iter() {
        if should_dismiss_menu_window(window.level()) {
            window.orderOut(None);
            continue;
        }
        if !should_raise_window(window.level(), &window.title().to_string()) {
            continue;
        }
        if window.isMiniaturized() {
            window.deminiaturize(None);
        }
        window.makeKeyAndOrderFront(None);
    }
}

/// The eframe document window. Popup menus share the app's window list and
/// must not be raised with it.
fn should_raise_window(level: NSWindowLevel, title: &str) -> bool {
    level == NSNormalWindowLevel && title == "Terminator"
}

fn should_dismiss_menu_window(level: NSWindowLevel) -> bool {
    level == NSPopUpMenuWindowLevel
}

struct Bar {
    item: Retained<NSStatusItem>,
    _target: Retained<StatusTarget>,
    png: Vec<u8>,
    menu_key: String,
    menu_ids: Vec<String>,
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

/// Rebuild the status menu only when the notice list changed. Menu tags
/// index [`Bar::menu_ids`]; GUI frames (the only writer) never run while
/// menu tracking holds the main thread, so a pick always resolves against
/// the list the menu was built from.
pub fn sync_status_menu(items: &[StatusMenuItem]) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    BAR.with(|slot| {
        let mut slot = slot.borrow_mut();
        let bar = slot.get_or_insert_with(|| install(mtm));
        let key = menu_key(items);
        if bar.menu_key == key {
            return;
        }
        bar.menu_key = key;
        bar.menu_ids = items.iter().map(|item| item.id.clone()).collect();
        rebuild_menu(mtm, bar, items);
    });
}

pub fn take_status_selection() -> Option<String> {
    SELECTION.lock().unwrap().take()
}

fn menu_key(items: &[StatusMenuItem]) -> String {
    let mut key = String::new();
    for item in items {
        key.push_str(&item.id);
        key.push('\n');
        key.push_str(&item.title);
        key.push('\n');
    }
    key
}

fn rebuild_menu(mtm: MainThreadMarker, bar: &Bar, items: &[StatusMenuItem]) {
    let menu = NSMenu::new(mtm);
    if items.is_empty() {
        let none = NSMenuItem::new(mtm);
        none.setTitle(ns_string!("No pending notifications"));
        none.setEnabled(false);
        menu.addItem(&none);
    } else {
        for (index, item) in items.iter().enumerate() {
            let title = NSString::from_str(&item.title);
            let entry = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &title,
                    Some(sel!(selectNotice:)),
                    &NSString::new(),
                )
            };
            entry.setTag(index as isize);
            unsafe {
                entry.setTarget(Some(&*bar._target));
            }
            menu.addItem(&entry);
        }
    }
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    let show = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            ns_string!("Show Terminator"),
            Some(sel!(showTerminator:)),
            &NSString::new(),
        )
    };
    unsafe {
        show.setTarget(Some(&*bar._target));
    }
    menu.addItem(&show);
    bar.item.setMenu(Some(&menu));
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
        menu_key: String::new(),
        menu_ids: Vec::new(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, title: &str) -> StatusMenuItem {
        StatusMenuItem {
            id: id.into(),
            title: title.into(),
        }
    }

    #[test]
    fn menu_key_changes_with_ids_titles_and_order() {
        let base = vec![item("a", "one"), item("b", "two")];
        assert_eq!(menu_key(&base), menu_key(&base));
        assert_ne!(menu_key(&base), menu_key(&[]));
        assert_ne!(
            menu_key(&base),
            menu_key(&[item("a", "one"), item("b", "three")])
        );
        assert_ne!(
            menu_key(&base),
            menu_key(&[item("b", "two"), item("a", "one")])
        );
    }

    #[test]
    fn only_the_document_window_is_raised() {
        assert!(should_raise_window(NSNormalWindowLevel, "Terminator"));
        assert!(!should_raise_window(NSPopUpMenuWindowLevel, "Terminator"));
        assert!(!should_raise_window(NSNormalWindowLevel, ""));
        assert!(should_dismiss_menu_window(NSPopUpMenuWindowLevel));
        assert!(!should_dismiss_menu_window(NSNormalWindowLevel));
    }
}
