use super::{Context, Duration, Path, Result, ensure, thread, wait};
use core_foundation::{
    base::CFType, boolean::CFBoolean, dictionary::CFDictionary, number::CFNumber,
};
use core_graphics::{
    event::{CGEvent, CGEventFlags, CGEventType, CGMouseButton},
    event_source::{CGEventSource, CGEventSourceStateID},
    geometry::CGPoint,
};
pub struct Desktop {
    pid: u32,
    window_id: u32,
}
fn value(dictionary: &CFDictionary, name: &str) -> Option<CFType> {
    terminator_sys::dictionary_value(dictionary, name)
}
fn number(dictionary: &CFDictionary, name: &str) -> Option<f64> {
    value(dictionary, name)?.downcast::<CFNumber>()?.to_f64()
}
impl Desktop {
    pub fn preflight() -> Result<()> {
        ensure!(
            terminator_sys::preflight_post_event_access(),
            "macOS Accessibility permission is required for this explicit native-input fixture; grant it to the terminal running cargo xtask, then rerun"
        );
        Ok(())
    }
    pub fn new(pid: u32, _output: &Path) -> Result<Self> {
        let mut desktop = Self { pid, window_id: 0 };
        if let Some(windows) = terminator_sys::window_dictionaries() {
            for dictionary in windows {
                if number(&dictionary, "kCGWindowOwnerPID") == Some(f64::from(pid)) {
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
            terminator_sys::window_dictionaries().context("Cannot read fixture window geometry")?;
        for dictionary in windows {
            if number(&dictionary, "kCGWindowOwnerPID") == Some(f64::from(self.pid))
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
        let pid = i32::try_from(self.pid).context("Fixture pid does not fit AX")?;
        terminator_sys::ax_primary_window(pid).map_err(anyhow::Error::msg)
    }
    fn ax_attribute(element: &CFType, name: &str) -> Result<CFType> {
        terminator_sys::ax_copy_attribute(element, name)
            .map_err(|_| anyhow::anyhow!("Missing native window attribute {name}"))
    }
    fn press_window_button(&self, name: &str) -> Result<()> {
        let window = self.ax_window()?;
        let button = Self::ax_attribute(&window, name)?;
        ensure!(
            terminator_sys::ax_perform(&button, "AXPress") == 0,
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
            terminator_sys::ax_set_attribute(&window, "AXMinimized", &CFBoolean::false_value())
                == 0,
            "Cannot restore fixture window"
        );
        let _ = terminator_sys::ax_perform(&window, "AXRaise");
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
