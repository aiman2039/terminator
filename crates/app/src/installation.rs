//! Validate the launch location before creating data or starting persistent PTYs.
use std::path::Path;

pub fn is_helper_error(error: &str) -> bool {
    error.starts_with("Attachment helper unavailable:")
}

pub fn attachment_helper(state: &terminator_core::State) -> anyhow::Result<std::path::PathBuf> {
    if state
        .capabilities
        .iter()
        .any(|c| c == terminator_core::STABLE_HELPER_CAPABILITY)
        && state.attachment_helper_available == Some(true)
        && let Some(path) = &state.attachment_helper_executable
        && path.is_absolute()
    {
        return Ok(path.clone());
    }
    // Old daemons don't advertise a private helper. A newly installed GUI's
    // bridge can still attach to their existing PTYs without replacing them.
    Ok(std::env::current_exe()?.with_file_name("terminator-hook"))
}

pub fn needs_installation(executable: &Path, home: Option<&Path>) -> bool {
    let Some(bundle) = executable
        .ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
    else {
        return false; // Unbundled development builds.
    };
    executable.starts_with("/Volumes")
        || executable
            .components()
            .any(|p| p.as_os_str() == "AppTranslocation")
        || !(bundle.starts_with("/Applications")
            || home.is_some_and(|h| bundle.starts_with(h.join("Applications"))))
}

pub fn preflight() -> anyhow::Result<bool> {
    let executable = std::env::current_exe()?;
    if cfg!(target_os = "macos")
        && needs_installation(
            &executable,
            std::env::var_os("HOME").as_deref().map(Path::new),
        )
    {
        if show_message(
            "Install Terminator before opening",
            "Drag Terminator from the disk image into Applications, eject the disk image, then open Terminator from Applications.\n\nThis keeps terminal sessions attached to a permanent installation. No daemon has been started by this launch.",
            true,
        ) {
            open::that("/Applications")?;
        }
        return Ok(false);
    }
    for name in ["terminator-daemon", "terminator-hook"] {
        let helper = executable.with_file_name(name);
        if !terminator_core::executable_available(&helper) {
            show_message(
                "Terminator installation needs repair",
                &format!(
                    "A required executable is missing or cannot be executed:\n{}\n\nReinstall the complete Terminator app into Applications. For a development build, run cargo build --workspace --locked. Existing terminal sessions have been preserved.",
                    helper.display()
                ),
                false,
            );
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(target_os = "macos")]
fn show_message(title: &str, description: &str, open_applications: bool) -> bool {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{
        NSAlert, NSAlertFirstButtonReturn, NSApplication, NSApplicationActivationPolicy,
    };
    use objc2_foundation::NSString;
    let mtm = MainThreadMarker::new().expect("Startup dialogs run on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    // An app-owned alert remains visible and associated with Terminator even
    // before winit has created its main window.
    {
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str(title));
        alert.setInformativeText(&NSString::from_str(description));
        alert.addButtonWithTitle(&NSString::from_str(if open_applications {
            "Open Applications"
        } else {
            "Quit"
        }));
        if open_applications {
            alert.addButtonWithTitle(&NSString::from_str("Quit"));
        }
        alert.runModal() == NSAlertFirstButtonReturn && open_applications
    }
}

#[cfg(not(target_os = "macos"))]
fn show_message(title: &str, description: &str, _open_applications: bool) -> bool {
    rfd::MessageDialog::new()
        .set_title(title)
        .set_description(description)
        .set_level(rfd::MessageLevel::Error)
        .show();
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gui_uses_only_an_advertised_available_private_helper() {
        use terminator_core::{STABLE_HELPER_CAPABILITY, State};
        let mut state = State {
            attachment_helper_executable: Some("/private/pinned/terminator-hook".into()),
            attachment_helper_available: Some(true),
            ..State::default()
        };
        let bundled = std::env::current_exe()
            .unwrap()
            .with_file_name("terminator-hook");
        assert_eq!(attachment_helper(&state).unwrap(), bundled);
        state.capabilities.push(STABLE_HELPER_CAPABILITY.into());
        assert_eq!(
            attachment_helper(&state).unwrap(),
            state.attachment_helper_executable.clone().unwrap()
        );
        state.attachment_helper_available = Some(false);
        assert_eq!(attachment_helper(&state).unwrap(), bundled);
        state.attachment_helper_available = Some(true);
        state.attachment_helper_executable = Some("relative/path".into());
        assert_eq!(attachment_helper(&state).unwrap(), bundled);
    }
    #[test]
    fn installer_downloaded_and_translocated_bundles_require_installation() {
        let home = Some(Path::new("/Users/test"));
        for path in [
            "/Volumes/Terminator/Terminator.app/Contents/MacOS/terminator",
            "/private/tmp/AppTranslocation/123/d/Terminator.app/Contents/MacOS/terminator",
            "/Users/test/Downloads/Terminator.app/Contents/MacOS/terminator",
            "/Applications-copy/Terminator.app/Contents/MacOS/terminator",
        ] {
            assert!(needs_installation(Path::new(path), home), "{path}");
        }
        for path in [
            "/Applications/Terminator.app/Contents/MacOS/terminator",
            "/Users/test/Applications/Terminator.app/Contents/MacOS/terminator",
            "/tmp/build/target/debug/terminator",
        ] {
            assert!(!needs_installation(Path::new(path), home), "{path}");
        }
    }
}
