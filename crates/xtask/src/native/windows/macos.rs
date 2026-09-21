use super::{Context, Duration, Path, Result, ensure, thread, wait};
use core_foundation::{
    base::{CFType, TCFType},
    boolean::CFBoolean,
    dictionary::CFDictionary,
    number::CFNumber,
    string::CFString,
};
use core_graphics::{
    event::{CGEvent, CGEventFlags, CGEventType, CGMouseButton},
    event_source::{CGEventSource, CGEventSourceStateID},
    geometry::CGPoint,
    window,
};
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightPostEventAccess() -> bool;
    fn AXUIElementCreateApplication(pid: i32) -> *const std::ffi::c_void;
    fn AXUIElementCopyAttributeValue(
        element: *const std::ffi::c_void,
        attribute: core_foundation::string::CFStringRef,
        value: *mut *const std::ffi::c_void,
    ) -> i32;
    fn AXUIElementSetAttributeValue(
        element: *const std::ffi::c_void,
        attribute: core_foundation::string::CFStringRef,
        value: *const std::ffi::c_void,
    ) -> i32;
    fn AXUIElementPerformAction(
        element: *const std::ffi::c_void,
        action: core_foundation::string::CFStringRef,
    ) -> i32;
}
pub struct Desktop {
    pid: u32,
    window_id: u32,
}
fn value(dictionary: &CFDictionary, name: &str) -> Option<CFType> {
    let key = CFString::new(name);
    let ptr = *dictionary.find(key.as_CFTypeRef())?;
    Some(unsafe { CFType::wrap_under_get_rule(ptr) })
}
fn number(dictionary: &CFDictionary, name: &str) -> Option<f64> {
    value(dictionary, name)?.downcast::<CFNumber>()?.to_f64()
}
impl Desktop {
    pub fn preflight() -> Result<()> {
        ensure!(
            unsafe { CGPreflightPostEventAccess() },
            "macOS Accessibility permission is required for this explicit native-input fixture; grant it to the terminal running cargo xtask, then rerun"
        );
        Ok(())
    }
    pub fn new(pid: u32, _output: &Path) -> Result<Self> {
        let mut desktop = Self { pid, window_id: 0 };
        if let Some(windows) =
            window::copy_window_info(window::kCGWindowListOptionAll, window::kCGNullWindowID)
        {
            for item in windows.iter() {
                let object = unsafe { CFType::wrap_under_get_rule(*item) };
                if let Some(dictionary) = object.downcast::<CFDictionary>()
                    && number(&dictionary, "kCGWindowOwnerPID") == Some(f64::from(pid))
                {
                    eprintln!(
                        "Mac fixture window: id={:?}, layer={:?}, visible={:?}, bounds={:?}",
                        number(&dictionary, "kCGWindowNumber"),
                        number(&dictionary, "kCGWindowLayer"),
                        value(&dictionary, "kCGWindowIsOnscreen"),
                        value(&dictionary, "kCGWindowBounds")
                    );
                }
            }
        }
        wait(|| Ok(desktop.geometry().is_ok()))?;
        desktop.window_id = number(&desktop.window()?, "kCGWindowNumber")
            .context("Missing native window ID")? as u32;
        Ok(desktop)
    }
    fn window(&self) -> Result<CFDictionary> {
        let windows =
            window::copy_window_info(window::kCGWindowListOptionAll, window::kCGNullWindowID)
                .context("Cannot read fixture window geometry")?;
        for item in windows.iter() {
            let object = unsafe { CFType::wrap_under_get_rule(*item) };
            if let Some(dictionary) = object.downcast::<CFDictionary>()
                && number(&dictionary, "kCGWindowOwnerPID") == Some(f64::from(self.pid))
                && number(&dictionary, "kCGWindowLayer").is_some_and(|n| n == 0.0 || n == 3.0)
                && (self.window_id == 0
                    || number(&dictionary, "kCGWindowNumber") == Some(f64::from(self.window_id)))
            {
                if self.window_id == 0
                    && !value(&dictionary, "kCGWindowIsOnscreen")
                        .and_then(|v| v.downcast::<CFBoolean>())
                        .is_some_and(bool::from)
                {
                    continue;
                }
                let Some(bounds) = value(&dictionary, "kCGWindowBounds")
                    .and_then(|v| v.downcast::<CFDictionary>())
                else {
                    continue;
                };
                if number(&bounds, "Width").unwrap_or(0.0) < 100.0 {
                    continue;
                }
                return Ok(dictionary);
            }
        }
        anyhow::bail!("Fixture window not found")
    }
    pub fn geometry(&self) -> Result<[f64; 4]> {
        let window = self.window()?;
        let bounds = value(&window, "kCGWindowBounds")
            .and_then(|v| v.downcast::<CFDictionary>())
            .context("Missing bounds")?;
        Ok([
            number(&bounds, "X").context("x")?,
            number(&bounds, "Y").context("y")?,
            number(&bounds, "Width").context("width")?,
            number(&bounds, "Height").context("height")?,
        ])
    }
    fn mouse(&self, kind: CGEventType, x: f64, y: f64) -> Result<()> {
        let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .map_err(|()| anyhow::anyhow!("Create event source"))?;
        let event = CGEvent::new_mouse_event(source, kind, CGPoint::new(x, y), CGMouseButton::Left)
            .map_err(|()| anyhow::anyhow!("Create mouse event"))?;
        event.set_integer_value_field(core_graphics::event::EventField::MOUSE_EVENT_CLICK_STATE, 1);
        event.set_integer_value_field(
            core_graphics::event::EventField::MOUSE_EVENT_WINDOW_UNDER_MOUSE_POINTER,
            i64::from(self.window_id),
        );
        event.set_integer_value_field(core_graphics::event::EventField::MOUSE_EVENT_WINDOW_UNDER_MOUSE_POINTER_THAT_CAN_HANDLE_THIS_EVENT,i64::from(self.window_id));
        if matches!(kind, CGEventType::LeftMouseDown) {
            let r = self.geometry()?;
            ensure!(
                x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3],
                "Refusing pointer press outside the fixture window"
            );
        }
        // Native resize/move gestures are handled by WindowServer, before the
        // event reaches the process. The verified fixture window is on top.
        event.post(core_graphics::event::CGEventTapLocation::HID);

        thread::sleep(Duration::from_millis(70));
        Ok(())
    }
    pub fn click(&mut self, x: f64, y: f64) -> Result<()> {
        self.mouse(CGEventType::MouseMoved, x, y)?;
        self.mouse(CGEventType::LeftMouseDown, x, y)?;
        self.mouse(CGEventType::LeftMouseUp, x, y)
    }
    pub fn drag(&mut self, start: (f64, f64), end: (f64, f64)) -> Result<()> {
        self.mouse(CGEventType::MouseMoved, start.0, start.1)?;
        self.mouse(CGEventType::LeftMouseDown, start.0, start.1)?;
        for i in 1..=10 {
            self.mouse(
                CGEventType::LeftMouseDragged,
                start.0 + (end.0 - start.0) * f64::from(i) / 10.0,
                start.1 + (end.1 - start.1) * f64::from(i) / 10.0,
            )?;
        }
        self.mouse(CGEventType::LeftMouseUp, end.0, end.1)
    }
    fn ax_window(&self) -> Result<CFType> {
        unsafe {
            let app = CFType::wrap_under_create_rule(AXUIElementCreateApplication(self.pid as i32));
            let mut result = std::ptr::null();
            ensure!(
                AXUIElementCopyAttributeValue(
                    app.as_CFTypeRef(),
                    CFString::new("AXWindows").as_concrete_TypeRef(),
                    &raw mut result
                ) == 0,
                "Cannot read fixture accessibility windows"
            );
            let windows = CFType::wrap_under_create_rule(result)
                .downcast::<core_foundation::array::CFArray>()
                .context("AX windows missing")?;
            let window = *windows.get(0).context("No fixture AX window")?;
            Ok(CFType::wrap_under_get_rule(window))
        }
    }
    fn ax_attribute(element: &CFType, name: &str) -> Result<CFType> {
        unsafe {
            let mut result = std::ptr::null();
            ensure!(
                AXUIElementCopyAttributeValue(
                    element.as_CFTypeRef(),
                    CFString::new(name).as_concrete_TypeRef(),
                    &raw mut result
                ) == 0,
                "Missing native window attribute {name}"
            );
            Ok(CFType::wrap_under_create_rule(result))
        }
    }
    fn press_window_button(&self, name: &str) -> Result<()> {
        let window = self.ax_window()?;
        let button = Self::ax_attribute(&window, name)?;
        ensure!(
            unsafe {
                AXUIElementPerformAction(
                    button.as_CFTypeRef(),
                    CFString::new("AXPress").as_concrete_TypeRef(),
                )
            } == 0,
            "Native button action failed"
        );
        Ok(())
    }
    pub fn minimize(&mut self) -> Result<()> {
        self.press_window_button("AXMinimizeButton")
    }
    pub fn minimized(&self) -> Result<bool> {
        let window = self.ax_window()?;
        let value = Self::ax_attribute(&window, "AXMinimized")?;
        Ok(value.downcast::<CFBoolean>().is_some_and(bool::from))
    }
    pub fn restore(&self) -> Result<()> {
        let window = self.ax_window()?;
        ensure!(
            unsafe {
                AXUIElementSetAttributeValue(
                    window.as_CFTypeRef(),
                    CFString::new("AXMinimized").as_concrete_TypeRef(),
                    CFBoolean::false_value().as_CFTypeRef(),
                )
            } == 0,
            "Cannot restore fixture window"
        );
        unsafe {
            AXUIElementPerformAction(
                window.as_CFTypeRef(),
                CFString::new("AXRaise").as_concrete_TypeRef(),
            );
        }
        Ok(())
    }
    pub fn close(&mut self) -> Result<()> {
        self.press_window_button("AXCloseButton")
    }
    fn key(&self, key: u16, flags: CGEventFlags, text: Option<&str>) -> Result<()> {
        for down in [true, false] {
            let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
                .map_err(|()| anyhow::anyhow!("Create event source"))?;
            let event = CGEvent::new_keyboard_event(source, key, down)
                .map_err(|()| anyhow::anyhow!("Create key event"))?;
            event.set_flags(flags);
            if down && let Some(text) = text {
                event.set_string(text);
            }
            event.post(core_graphics::event::CGEventTapLocation::HID);
            thread::sleep(Duration::from_millis(60));
        }
        Ok(())
    }
    pub fn open_file_shortcut(&self) -> Result<()> {
        self.key(31, CGEventFlags::CGEventFlagCommand, None)
    }
    pub fn escape(&self) -> Result<()> {
        self.key(53, CGEventFlags::empty(), None)
    }
    pub fn choose_path(&self, path: &str) -> Result<()> {
        self.key(
            5,
            CGEventFlags::CGEventFlagCommand | CGEventFlags::CGEventFlagShift,
            None,
        )?;
        thread::sleep(Duration::from_millis(250));
        self.key(0, CGEventFlags::empty(), Some(path))?;
        self.key(36, CGEventFlags::empty(), None)?;
        thread::sleep(Duration::from_millis(500));
        self.key(36, CGEventFlags::empty(), None)
    }
}
