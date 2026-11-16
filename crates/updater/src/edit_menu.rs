//! macOS Edit menu. WKWebView (Browser and drawing tabs) runs cut, copy,
//! paste and select all only when a main-menu item sends the action, so
//! without this menu Cmd+C/X/V/A do nothing in a web page. The items have
//! no target: AppKit disables them while the egui view has focus, and the
//! key then goes to egui as before.

use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{NSApplication, NSMenu, NSMenuItem};
use objc2_foundation::ns_string;

/// Adds the Edit menu after the application menu. Does nothing when the
/// application has no main menu.
pub(crate) fn install(mtm: MainThreadMarker) {
    let Some(main) = NSApplication::sharedApplication(mtm).mainMenu() else {
        return;
    };
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), ns_string!("Edit"));
    let items = [
        (ns_string!("Cut"), sel!(cut:), ns_string!("x")),
        (ns_string!("Copy"), sel!(copy:), ns_string!("c")),
        (ns_string!("Paste"), sel!(paste:), ns_string!("v")),
        (ns_string!("Select All"), sel!(selectAll:), ns_string!("a")),
    ];
    for (title, action, key) in items {
        // SAFETY: The item has no target, so the action goes to the first
        // responder that implements it.
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                title,
                Some(action),
                key,
            )
        };
        menu.addItem(&item);
    }
    let holder = NSMenuItem::new(mtm);
    holder.setSubmenu(Some(&menu));
    main.insertItem_atIndex(&holder, isize::from(main.numberOfItems() > 0));
}
