//! Quit handling: `-[NSApplication terminate:]` is swizzled so the GUI can
//! checkpoint before AppKit exits.
use super::*;
use std::sync::{
    OnceLock,
    atomic::{AtomicBool, Ordering},
};

static REQUESTED: AtomicBool = AtomicBool::new(false);
pub(super) static ORIGINAL: OnceLock<Imp> = OnceLock::new();
static CANCELLED: AtomicBool = AtomicBool::new(false);
pub(super) static CONTEXT: OnceLock<egui::Context> = OnceLock::new();

pub(super) unsafe extern "C-unwind" fn terminate(
    _application: *mut AnyObject,
    _sel: Sel,
    _sender: *mut AnyObject,
) {
    // Do not enter AppKit's NSTerminateLater modal loop: it prevents winit
    // from applying the queued GUI results needed for our checkpoint.
    REQUESTED.store(true, Ordering::Release);
    if let Some(ctx) = CONTEXT.get() {
        ctx.request_repaint();
    }
}
#[cfg(feature = "test-support")]
pub fn fixture_native_quit() {
    dispatch2::DispatchQueue::main().exec_async(|| {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        NSApplication::sharedApplication(mtm).terminate(None);
    });
}
pub fn termination_requested() -> bool {
    REQUESTED.swap(false, Ordering::AcqRel)
}
pub fn termination_cancelled() -> bool {
    CANCELLED.swap(false, Ordering::AcqRel)
}
pub fn cancel_termination() {
    REQUESTED.store(false, Ordering::Release);
}
pub fn complete_termination(_ctx: &egui::Context) {
    if cfg!(test) {
        return;
    }
    // AppKit broadcasts applicationWillTerminate synchronously. Dispatch
    // outside the egui/winit callback to avoid re-entering winit's handler.
    dispatch2::DispatchQueue::main().exec_async(|| {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        if let Some(original) = ORIGINAL.get() {
            // Exact ABI of -[NSApplication terminate:]. Calling the saved
            // implementation preserves AppKit and Sparkle quit observers.
            // AppKit exits inside that call. Returning means the process
            // is still alive, so Sparkle's Install and Relaunch waits forever.
            // SAFETY: `original` is the IMP saved from `-[NSApplication terminate:]`.
            // The signature matches that method. `app` is the shared application and
            // outlives the call. `terminate:` accepts a nil sender.
            unsafe {
                let original: unsafe extern "C-unwind" fn(
                    *const NSApplication,
                    Sel,
                    *const AnyObject,
                ) = std::mem::transmute(*original);
                original(&raw const *app, sel!(terminate:), std::ptr::null());
            }
        }
        std::process::exit(0);
    });
}
