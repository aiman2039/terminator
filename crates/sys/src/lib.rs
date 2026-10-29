//! OS calls that still need `unsafe`: `pre_exec`, macOS `proc_pidinfo`, macOS memory
//! pressure (`host_statistics64`, `sysctlbyname`), Accessibility, and the
//! memory-mapped VNC framebuffer shared by the daemon and the GUI.
//! Sparkle stays in `terminator-updater`. Every other Terminator crate forbids `unsafe`.

#[cfg(windows)]
mod console;
#[cfg(windows)]
mod handle;
mod pressure;
mod session;
mod shm;

#[cfg(target_os = "macos")]
mod threads;

#[cfg(windows)]
pub use console::{RawMode, dimensions};
#[cfg(windows)]
pub use handle::socket_into_handle;
pub use pressure::{MemoryPressure, PressureLevel, memory_pressure};
pub use session::{detach_session, double_fork_setsid};
pub use shm::{FrameRect, SharedFrameReader, SharedFrameWriter};

#[cfg(target_os = "macos")]
pub use threads::all_threads_waiting;

#[cfg(all(target_os = "macos", feature = "desktop"))]
mod desktop;
#[cfg(all(target_os = "macos", feature = "desktop"))]
pub use desktop::{
    ax_copy_attribute, ax_perform, ax_primary_window, ax_set_attribute, dictionary_value,
    preflight_post_event_access, window_dictionaries,
};
