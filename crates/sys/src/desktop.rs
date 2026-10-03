use core_foundation::{
    array::CFArray,
    base::{CFType, TCFType},
    dictionary::CFDictionary,
    string::CFString,
};
use std::ffi::c_void;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightPostEventAccess() -> bool;
    fn AXUIElementCreateApplication(pid: i32) -> *const c_void;
    fn AXUIElementCopyAttributeValue(
        element: *const c_void,
        attribute: core_foundation::string::CFStringRef,
        value: *mut *const c_void,
    ) -> i32;
    fn AXUIElementSetAttributeValue(
        element: *const c_void,
        attribute: core_foundation::string::CFStringRef,
        value: *const c_void,
    ) -> i32;
    fn AXUIElementPerformAction(
        element: *const c_void,
        action: core_foundation::string::CFStringRef,
    ) -> i32;
}

pub fn preflight_post_event_access() -> bool {
    // SAFETY: `CGPreflightPostEventAccess` takes no pointers and only returns a bool.
    unsafe { CGPreflightPostEventAccess() }
}

pub fn dictionary_value(dictionary: &CFDictionary, name: &str) -> Option<CFType> {
    let key = CFString::new(name);
    let ptr = *dictionary.find(key.as_CFTypeRef())?;
    // SAFETY: `find` borrows a value owned by `dictionary`. The get rule retains it for the caller.
    Some(unsafe { CFType::wrap_under_get_rule(ptr) })
}

pub fn window_dictionaries() -> Option<Vec<CFDictionary>> {
    let windows = core_graphics::window::copy_window_info(
        core_graphics::window::kCGWindowListOptionAll,
        core_graphics::window::kCGNullWindowID,
    )?;
    let mut dictionaries = Vec::new();
    for item in windows.iter() {
        // SAFETY: Window-list entries are get-rule CoreFoundation values. Retain this one.
        let object = unsafe { CFType::wrap_under_get_rule(*item) };
        if let Some(dictionary) = object.downcast::<CFDictionary>() {
            dictionaries.push(dictionary);
        }
    }
    Some(dictionaries)
}

fn ax_application(pid: i32) -> CFType {
    // SAFETY: `AXUIElementCreateApplication` returns +1. The create rule takes that ownership.
    unsafe { CFType::wrap_under_create_rule(AXUIElementCreateApplication(pid)) }
}

pub fn ax_copy_attribute(element: &CFType, name: &str) -> Result<CFType, i32> {
    // SAFETY: `element` and the attribute name are valid CoreFoundation refs for the call.
    // On success the out-value is +1 and the create rule takes it. On failure it is not wrapped.
    unsafe {
        let mut result = std::ptr::null();
        let status = AXUIElementCopyAttributeValue(
            element.as_CFTypeRef(),
            CFString::new(name).as_concrete_TypeRef(),
            &raw mut result,
        );
        if status != 0 {
            return Err(status);
        }
        Ok(CFType::wrap_under_create_rule(result))
    }
}

pub fn ax_set_attribute(element: &CFType, name: &str, value: &impl TCFType) -> i32 {
    // SAFETY: `element`, the attribute name, and `value` are valid CoreFoundation refs.
    // The call does not transfer ownership; it only returns a status code.
    unsafe {
        AXUIElementSetAttributeValue(
            element.as_CFTypeRef(),
            CFString::new(name).as_concrete_TypeRef(),
            value.as_CFTypeRef(),
        )
    }
}

pub fn ax_perform(element: &CFType, action: &str) -> i32 {
    // SAFETY: `element` and the action name are valid CoreFoundation refs. The call only returns a status.
    unsafe {
        AXUIElementPerformAction(
            element.as_CFTypeRef(),
            CFString::new(action).as_concrete_TypeRef(),
        )
    }
}

pub fn ax_primary_window(pid: i32) -> Result<CFType, String> {
    let app = ax_application(pid);
    let windows = ax_copy_attribute(&app, "AXWindows")
        .map_err(|_| "Cannot read fixture accessibility windows".to_string())?;
    let windows = windows
        .downcast::<CFArray>()
        .ok_or_else(|| "AX windows missing".to_string())?;
    let raw = windows
        .get(0)
        .ok_or_else(|| "No fixture AX window".to_string())?;
    // SAFETY: The array retains its elements. The get rule retains this one for the caller.
    unsafe { Ok(CFType::wrap_under_get_rule(*raw)) }
}
