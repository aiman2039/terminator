//! Windows console attachment: window size and raw input mode.
//!
//! The attach bridge needs the console window size (like `TIOCGWINSZ`) and a
//! raw input mode (like termios raw) so full-screen programs receive keys
//! unprocessed. Implemented against the Console API directly.

#[cfg(windows)]
use std::io;
#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::INVALID_HANDLE_VALUE,
    System::Console::{
        CONSOLE_SCREEN_BUFFER_INFO, ENABLE_EXTENDED_FLAGS, ENABLE_INSERT_MODE, ENABLE_LINE_INPUT,
        ENABLE_PROCESSED_INPUT, ENABLE_PROCESSED_OUTPUT, ENABLE_QUICK_EDIT_MODE,
        ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING, ENABLE_WINDOW_INPUT,
        ENABLE_WRAP_AT_EOL_OUTPUT, GetConsoleMode, GetConsoleScreenBufferInfo, GetStdHandle,
        STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetConsoleMode,
    },
};

/// Console window size in `(rows, cols)`, falling back to 24x80 when the
/// standard handles are redirected or unavailable.
#[cfg(windows)]
#[must_use]
pub fn dimensions() -> (u16, u16) {
    // SAFETY: `info` is a plain data struct written only by the OS call whose
    // return value is checked before any field is read.
    unsafe {
        let output = GetStdHandle(STD_OUTPUT_HANDLE);
        if output == INVALID_HANDLE_VALUE || output.is_null() {
            return (24, 80);
        }
        let mut info = std::mem::zeroed::<CONSOLE_SCREEN_BUFFER_INFO>();
        if GetConsoleScreenBufferInfo(output, std::ptr::addr_of_mut!(info)) == 0 {
            return (24, 80);
        }
        let window = info.srWindow;
        // Clamped to at least 1 above, so every value fits `u16`.
        let cols = u16::try_from(
            window
                .Right
                .saturating_sub(window.Left)
                .saturating_add(1)
                .max(1),
        )
        .unwrap_or(u16::MAX);
        let rows = u16::try_from(
            window
                .Bottom
                .saturating_sub(window.Top)
                .saturating_add(1)
                .max(1),
        )
        .unwrap_or(u16::MAX);
        (rows, cols)
    }
}

/// Raw console input/output modes. Restores the previous modes on drop.
#[cfg(windows)]
pub struct RawMode {
    input: *mut std::ffi::c_void,
    prev_input: u32,
    output: *mut std::ffi::c_void,
    prev_output: u32,
}

#[cfg(windows)]
impl RawMode {
    /// Disable line input, echo, and processed input; enable virtual-terminal
    /// input plus window events (the resize signal) and VT output processing.
    pub fn enable() -> io::Result<Self> {
        // SAFETY: handle/mode pairs come from `GetStdHandle`/`GetConsoleMode`
        // and are restored on drop; every OS return value is checked.
        unsafe {
            let input = GetStdHandle(STD_INPUT_HANDLE);
            let output = GetStdHandle(STD_OUTPUT_HANDLE);
            if input == INVALID_HANDLE_VALUE
                || input.is_null()
                || output == INVALID_HANDLE_VALUE
                || output.is_null()
            {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "console handles are redirected",
                ));
            }
            let mut prev_input = 0;
            let mut prev_output = 0;
            if GetConsoleMode(input, std::ptr::addr_of_mut!(prev_input)) == 0
                || GetConsoleMode(output, std::ptr::addr_of_mut!(prev_output)) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let raw_input = (prev_input
                & !(ENABLE_LINE_INPUT
                    | ENABLE_PROCESSED_INPUT
                    | ENABLE_QUICK_EDIT_MODE
                    | ENABLE_INSERT_MODE))
                | ENABLE_VIRTUAL_TERMINAL_INPUT
                | ENABLE_WINDOW_INPUT
                | ENABLE_EXTENDED_FLAGS;
            let raw_output = (prev_output | ENABLE_PROCESSED_OUTPUT)
                | ENABLE_WRAP_AT_EOL_OUTPUT
                | ENABLE_VIRTUAL_TERMINAL_PROCESSING;
            if SetConsoleMode(input, raw_input) == 0 || SetConsoleMode(output, raw_output) == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                input,
                prev_input,
                output,
                prev_output,
            })
        }
    }
}

#[cfg(windows)]
impl Drop for RawMode {
    fn drop(&mut self) {
        // SAFETY: restores modes saved by `enable` on the same handles; the
        // process is past setup, so a failure here is unrecoverable anyway.
        unsafe {
            let _ = SetConsoleMode(self.input, self.prev_input);
            let _ = SetConsoleMode(self.output, self.prev_output);
        }
    }
}
