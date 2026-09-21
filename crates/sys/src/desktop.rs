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
    unsafe { CGPreflightPostEventAccess() }
}

pub fn dictionary_value(dictionary: &CFDictionary, name: &str) -> Option<CFType> {
    let key = CFString::new(name);
    let ptr = *dictionary.find(key.as_CFTypeRef())?;
    // `find` borrows a value owned by `dictionary`. Retain it for the caller.
    Some(unsafe { CFType::wrap_under_get_rule(ptr) })
}

pub fn window_dictionaries() -> Option<Vec<CFDictionary>> {
    let windows = core_graphics::window::copy_window_info(
        core_graphics::window::kCGWindowListOptionAll,
        core_graphics::window::kCGNullWindowID,
    )?;
    let mut dictionaries = Vec::new();
    for item in windows.iter() {
        // Window-list entries are get-rule CoreFoundation values.
        let object = unsafe { CFType::wrap_under_get_rule(*item) };
        if let Some(dictionary) = object.downcast::<CFDictionary>() {
            dictionaries.push(dictionary);
        }
    }
    Some(dictionaries)
}

fn ax_application(pid: i32) -> CFType {
    unsafe { CFType::wrap_under_create_rule(AXUIElementCreateApplication(pid)) }
}

pub fn ax_copy_attribute(element: &CFType, name: &str) -> Result<CFType, i32> {
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
    unsafe {
        AXUIElementSetAttributeValue(
            element.as_CFTypeRef(),
            CFString::new(name).as_concrete_TypeRef(),
            value.as_CFTypeRef(),
        )
    }
}

pub fn ax_perform(element: &CFType, action: &str) -> i32 {
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
    // The array retains its elements. Retain this one for the caller.
    unsafe { Ok(CFType::wrap_under_get_rule(*raw)) }
}
