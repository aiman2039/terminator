use std::{
    process::{Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};
use terminator_core::{Paths, atomic_write};
static ACTIVE: AtomicUsize = AtomicUsize::new(0);

pub struct DesktopAlert {
    pub paths: Paths,
    pub summary: String,
    pub notice: String,
    pub sound: bool,
}

pub fn initialize() {
    #[cfg(target_os = "macos")]
    {
        let _ = notify_rust::set_application("dev.terminator.app");
    }
}

fn sound_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "Glass"
    } else {
        "message-new-instant"
    }
}

pub fn send(
    DesktopAlert {
        paths,
        summary,
        notice,
        sound,
    }: DesktopAlert,
) {
    if ACTIVE.fetch_add(1, Ordering::Relaxed) >= 16 {
        ACTIVE.fetch_sub(1, Ordering::Relaxed);
        return;
    }
    std::thread::spawn(move || {
        let mut notification = notify_rust::Notification::new();
        notification
            .summary(&summary)
            .body("Open the notification to view the session context.")
            .appname("Terminator")
            .action("default", "Open context")
            .timeout(10000);
        if sound {
            notification.sound_name(sound_name());
        }
        if let Ok(handle) = notification.show() {
            handle.wait_for_action(|action| {
                if action == "__closed" {
                    return;
                }
                let _ = atomic_write(&paths.runtime.join("activation"), notice.as_bytes());
                if let Ok(exe) = std::env::var_os("TERMINATOR_GUI_EXECUTABLE")
                    .map(std::path::PathBuf::from)
                    .map(Ok)
                    .unwrap_or_else(std::env::current_exe)
                {
                    #[cfg(target_os = "macos")]
                    let mut command = {
                        let mut c = Command::new("open");
                        if let Some(bundle) = exe
                            .parent()
                            .and_then(|p| p.parent())
                            .and_then(|p| p.parent())
                            .filter(|p| p.extension().is_some_and(|e| e == "app"))
                        {
                            c.arg("-a")
                                .arg(bundle)
                                .args(["--args", "--data-dir"])
                                .arg(&paths.data);
                        } else {
                            c.arg("-a").arg(exe.with_file_name("terminator"));
                        }
                        c
                    };
                    #[cfg(not(target_os = "macos"))]
                    let mut command = {
                        let mut c = Command::new(exe.with_file_name("terminator"));
                        c.arg("--data-dir").arg(&paths.data);
                        c
                    };
                    command
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null());
                    if let Ok(mut child) = command.spawn() {
                        let _ = child.wait();
                    }
                }
            });
        }
        ACTIVE.fetch_sub(1, Ordering::Relaxed);
    });
}
/// Cocoa delivers notification callbacks on the daemon main thread.
pub fn idle() {
    let started = std::time::Instant::now();
    #[cfg(target_os = "macos")]
    unsafe {
        use std::ffi::c_void;
        #[link(name = "CoreFoundation", kind = "framework")]
        unsafe extern "C" {
            static kCFRunLoopDefaultMode: *const c_void;
            fn CFRunLoopRunInMode(
                mode: *const c_void,
                seconds: f64,
                return_after_source_handled: bool,
            ) -> i32;
        }
        CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.01, true);
    }
    std::thread::sleep(std::time::Duration::from_millis(10).saturating_sub(started.elapsed()));
}

#[cfg(test)]
mod tests {
    #[test]
    fn os_banner_sound_name_is_platform_default() {
        let name = super::sound_name();
        assert!(!name.is_empty());
        if cfg!(target_os = "macos") {
            assert_eq!(name, "Glass");
        } else {
            assert_eq!(name, "message-new-instant");
        }
    }
}
