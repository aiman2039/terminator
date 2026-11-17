//! Terminator addition: the native window of each live viewport, so the
//! app can mount a native child view (webview) in a floating window.
//! See `UPSTREAM.md`.

use std::{cell::RefCell, collections::HashMap};

use egui::ViewportId;
use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, WindowHandle};
use winit::window::Window;

thread_local! {
    /// Window handles are main-thread values; so is this registry.
    static WINDOWS: RefCell<HashMap<ViewportId, RawWindowHandle>> = RefCell::default();
}

/// The native window of one viewport, as recorded at its latest input pass.
#[derive(Clone, Copy, Debug)]
pub struct ViewportWindow(RawWindowHandle);

impl HasWindowHandle for ViewportWindow {
    #[allow(unsafe_code)]
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        // SAFETY: the handle was read from a live winit window and its
        // registry entry is removed when that viewport's `State` drops,
        // which the integration does together with the window. Callers get
        // a `ViewportWindow` only from `viewport_window` and use it in the
        // same frame on the main thread.
        Ok(unsafe { WindowHandle::borrow_raw(self.0) })
    }
}

/// The window of `viewport`, if that viewport has taken input and is alive.
pub fn viewport_window(viewport: ViewportId) -> Option<ViewportWindow> {
    WINDOWS.with_borrow(|windows| windows.get(&viewport).copied().map(ViewportWindow))
}

pub(crate) fn record(viewport: ViewportId, window: &Window) {
    if let Ok(handle) = window.window_handle() {
        WINDOWS.with_borrow_mut(|windows| {
            windows.insert(viewport, handle.as_raw());
        });
    }
}

pub(crate) fn forget(viewport: ViewportId) {
    // `try_with`: a `State` can drop while the thread tears down.
    let _ = WINDOWS.try_with(|windows| {
        if let Ok(mut windows) = windows.try_borrow_mut() {
            windows.remove(&viewport);
        }
    });
}
