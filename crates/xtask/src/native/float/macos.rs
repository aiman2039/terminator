use anyhow::{Context, Result, ensure};
use std::{fs, path::PathBuf, process::Command};

pub(super) fn float_window_id(pid: u32, title: &str) -> Option<u32> {
    use core_foundation::{boolean::CFBoolean, number::CFNumber, string::CFString};
    // NB: skip entries with missing keys instead of bailing: window-list
    // dictionaries for shadow/menu helpers routinely lack IsOnscreen or a
    // name, and `?` here used to abort the whole search before reaching
    // our window.
    let windows = terminator_sys::window_dictionaries()?;
    for dictionary in windows {
        let owner = terminator_sys::dictionary_value(&dictionary, "kCGWindowOwnerPID")
            .and_then(|value| value.downcast::<CFNumber>())
            .and_then(|number| number.to_f64());
        if owner != Some(f64::from(pid)) {
            continue;
        }
        let onscreen = terminator_sys::dictionary_value(&dictionary, "kCGWindowIsOnscreen")
            .and_then(|value| value.downcast::<CFBoolean>())
            .map(bool::from);
        if onscreen != Some(true) {
            continue;
        }
        let name: Option<String> = terminator_sys::dictionary_value(&dictionary, "kCGWindowName")
            .and_then(|value| value.downcast::<CFString>())
            .map(|name| name.to_string());
        if name.as_deref() != Some(title) {
            continue;
        }
        return terminator_sys::dictionary_value(&dictionary, "kCGWindowNumber")
            .and_then(|value| value.downcast::<CFNumber>())
            .and_then(|number| number.to_f64())
            .and_then(|id| {
                // Window numbers are small positive integers; the range
                // check leaves nothing to truncate or lose the sign of.
                // (`as` saturates, so the eager narrow is harmless.)
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "Range checked on the next line"
                )]
                let narrowed = id as u32;
                (id >= 0.0 && id <= f64::from(u32::MAX)).then_some(narrowed)
            });
    }
    None
}

pub(super) fn pid_windows(pid: u32) -> Vec<String> {
    use core_foundation::{boolean::CFBoolean, number::CFNumber, string::CFString};
    let mut out = Vec::new();
    let Some(windows) = terminator_sys::window_dictionaries() else {
        return out;
    };
    for dictionary in windows {
        let owner = terminator_sys::dictionary_value(&dictionary, "kCGWindowOwnerPID")
            .and_then(|value| value.downcast::<CFNumber>())
            .and_then(|number| number.to_f64());
        if owner != Some(f64::from(pid)) {
            continue;
        }
        let name: Option<String> = terminator_sys::dictionary_value(&dictionary, "kCGWindowName")
            .and_then(|value| value.downcast::<CFString>())
            .map(|name| name.to_string());
        let onscreen: Option<bool> =
            terminator_sys::dictionary_value(&dictionary, "kCGWindowIsOnscreen")
                .and_then(|value| value.downcast::<CFBoolean>())
                .map(bool::from);
        let number: Option<f64> = terminator_sys::dictionary_value(&dictionary, "kCGWindowNumber")
            .and_then(|value| value.downcast::<CFNumber>())
            .and_then(|number| number.to_f64());
        out.push(format!(
            "name={name:?} onscreen={onscreen:?} number={number:?}"
        ));
    }
    out
}

pub(super) fn windows_named(name: &str) -> Vec<String> {
    use core_foundation::{boolean::CFBoolean, number::CFNumber, string::CFString};
    let mut out = Vec::new();
    let Some(windows) = terminator_sys::window_dictionaries() else {
        out.push("window list unavailable".to_owned());
        return out;
    };
    for dictionary in windows {
        let title: Option<String> = terminator_sys::dictionary_value(&dictionary, "kCGWindowName")
            .and_then(|value| value.downcast::<CFString>())
            .map(|title| title.to_string());
        if title.as_deref() != Some(name) {
            continue;
        }
        let owner: Option<f64> = terminator_sys::dictionary_value(&dictionary, "kCGWindowOwnerPID")
            .and_then(|value| value.downcast::<CFNumber>())
            .and_then(|number| number.to_f64());
        let onscreen: Option<bool> =
            terminator_sys::dictionary_value(&dictionary, "kCGWindowIsOnscreen")
                .and_then(|value| value.downcast::<CFBoolean>())
                .map(bool::from);
        out.push(format!("owner={owner:?} onscreen={onscreen:?}"));
    }
    out
}

pub(super) fn capture_window(id: u32, path: &PathBuf) -> Result<()> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    let status = Command::new("screencapture")
        .args(["-x", "-l", &id.to_string()])
        .arg(path)
        .stderr(std::process::Stdio::null())
        .status()
        .context("screencapture failed")?;
    ensure!(status.success(), "screencapture failed");
    let bytes = path.metadata()?.len();
    ensure!(bytes > 1000, "float capture too small ({bytes} bytes)");
    Ok(())
}
