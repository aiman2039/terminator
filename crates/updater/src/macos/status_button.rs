//! Sparkle's install status button, rebound to the control the user sees.
use super::*;
use std::sync::OnceLock;

/// Sparkle 2.10 sizes the status button with `setFrame` while Auto Layout
/// owns it (`translatesAutoresizingMaskIntoConstraints = NO`). On macOS 26
/// that frame is not the control the user sees, so Install and Relaunch
/// can ignore the click. Keep the title binding and point the real button
/// at the action; leave the constraints in place.
pub(super) fn keep_sparkle_status_button_in_layout() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    // SAFETY: Replaces Sparkle's private
    // `setButtonTitle:target:action:isDefault:accessibilityIdentifier:` once.
    // `set_status_button` uses that exact signature.
    INSTALLED.get_or_init(|| unsafe {
        let Some(class) = AnyClass::get(c"SUStatusController") else {
            return;
        };
        let Some(method) = class
            .instance_method(sel!(setButtonTitle:target:action:isDefault:accessibilityIdentifier:))
        else {
            return;
        };
        method.set_implementation(std::mem::transmute::<
            unsafe extern "C-unwind" fn(
                *mut AnyObject,
                Sel,
                *const NSString,
                *mut AnyObject,
                Option<Sel>,
                Bool,
                *const NSString,
            ),
            Imp,
        >(set_status_button));
    });
}

unsafe extern "C-unwind" fn set_status_button(
    this: *mut AnyObject,
    _cmd: Sel,
    title: *const NSString,
    target: *mut AnyObject,
    action: Option<Sel>,
    is_default: Bool,
    identifier: *const NSString,
) {
    use objc2_app_kit::{NSAccessibility, NSButton, NSWindowController};
    if this.is_null() {
        return;
    }
    // SAFETY: `this` was null-checked. Sparkle passes the SUStatusController for this call.
    let this = unsafe { &*this };
    if !title.is_null() {
        // SAFETY: `title` was null-checked. Sparkle passes the button title NSString.
        let title = unsafe { &*title };
        // SAFETY: `setButtonTitle:` is Sparkle's private NSString setter. `title` is valid for the call.
        let _: () = unsafe { msg_send![this, setButtonTitle: title] };
    }
    // Sparkle 2.10: SUStatusController is an NSWindowController.
    let Some(content) = this
        .downcast_ref::<NSWindowController>()
        .and_then(objc2_app_kit::NSWindowController::window)
        .and_then(|window| window.contentView())
    else {
        return;
    };
    let subviews = content.subviews();
    let mut button = None;
    for index in 0..subviews.count() {
        if let Ok(found) = subviews.objectAtIndex(index).downcast::<NSButton>() {
            button = Some(found);
            break;
        }
    }
    let Some(button) = button else {
        return;
    };
    // SAFETY: `button` is the first NSButton in the status window. `target` is
    // null-checked before the dereference. `action` is the selector Sparkle passed.
    unsafe {
        button.setTarget((!target.is_null()).then(|| &*target));
        button.setAction(action);
    }
    let key = if is_default.as_bool() {
        ns_string!("\r")
    } else {
        ns_string!("")
    };
    button.setKeyEquivalent(key);
    if !identifier.is_null() {
        // SAFETY: `identifier` was null-checked. Sparkle passes the accessibility NSString.
        let identifier = unsafe { &*identifier };
        button.setAccessibilityIdentifier(Some(identifier));
    }
    button.setEnabled(!target.is_null());
    content.layoutSubtreeIfNeeded();
}
