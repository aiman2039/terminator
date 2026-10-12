//! Image-only clipboard pastes are saved on the worker, never during rendering.
use anyhow::{Context, Result, ensure};
use std::path::Path;

pub fn read_paste() -> Result<Option<String>> {
    let mut clipboard = arboard::Clipboard::new().context("Cannot open clipboard")?;
    // Match Orca: text wins when both text and image formats are available.
    if let Ok(text) = clipboard.get_text()
        && !text.is_empty()
    {
        return Ok(Some(text.replace("\r\n", "\n")));
    }
    match clipboard.get_image() {
        Ok(image) => save_image(&image, &std::env::temp_dir()).map(Some),
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(error) => Err(error).context("Cannot read clipboard image"),
    }
}

fn save_image(image: &arboard::ImageData<'_>, directory: &Path) -> Result<String> {
    let bytes = image
        .width
        .checked_mul(image.height)
        .and_then(|n| n.checked_mul(4));
    ensure!(
        image.width > 0
            && image.height > 0
            && bytes == Some(image.bytes.len())
            && image.bytes.len() <= 128 * 1024 * 1024,
        "Clipboard image is invalid or exceeds 128 MiB"
    );
    let width = u32::try_from(image.width)?;
    let height = u32::try_from(image.height)?;
    // NamedTempFile creates a private, exclusive file (0600 on Unix).
    let mut file = tempfile::Builder::new()
        .prefix("terminator-paste-")
        .suffix(".png")
        .tempfile_in(directory)
        .context("Cannot create clipboard image file")?;
    image::write_buffer_with_format(
        &mut file,
        &image.bytes,
        width,
        height,
        image::ExtendedColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .context("Cannot encode clipboard image")?;
    // Keep it after GUI exit: daemon-owned agents may read the path later.
    // The OS owns eventual temp-directory cleanup.
    let (_, path) = file.keep().context("Cannot retain clipboard image file")?;
    Ok(terminator_core::quote(&path.to_string_lossy()))
}

pub fn take_image_paste(
    events: &mut Vec<eframe::egui::Event>,
    modifiers: eframe::egui::Modifiers,
) -> usize {
    // On Linux ordinary Ctrl+V remains terminal input (^V).
    if !cfg!(target_os = "macos") && !(modifiers.command && modifiers.shift) {
        return 0;
    }
    let mut count: usize = 0;
    events.retain(|event| {
        if matches!(event, eframe::egui::Event::Paste(text) if text.is_empty()) {
            count = count.saturating_add(1);
            false
        } else {
            true
        }
    });
    count
}

fn copy_key(event: &eframe::egui::Event) -> bool {
    use eframe::egui::{Event, Key};
    matches!(event, Event::Key { key: Key::C, modifiers, .. }
        if !modifiers.alt && (modifiers.ctrl || modifiers.command || modifiers.mac_cmd))
}

pub fn terminal_copy_requested(
    events: &[eframe::egui::Event],
    modifiers: eframe::egui::Modifiers,
) -> bool {
    use eframe::egui::Event;
    events.iter().any(|event| {
        matches!(event, Event::Copy)
            || matches!(event, Event::Key { pressed: true, .. } if copy_key(event))
            || matches!(event, Event::Text(text) if text == "\u{3}" && !modifiers.alt)
    })
}

/// Consume every representation of a copy chord only when text is available.
/// In particular, Ctrl+C must not also deliver ETX to the shell.
pub fn take_terminal_copy(
    events: &mut Vec<eframe::egui::Event>,
    modifiers: eframe::egui::Modifiers,
    selection: &str,
) -> Option<String> {
    use eframe::egui::Event;
    if selection.is_empty() || !terminal_copy_requested(events, modifiers) {
        return None;
    }
    let control = modifiers.ctrl
        || events.iter().any(|event| {
            matches!(event, Event::Key { modifiers, .. } if copy_key(event) && modifiers.ctrl)
        });
    events.retain(|event| {
        !matches!(event, Event::Copy)
            && !copy_key(event)
            && !matches!(event, Event::Text(text)
                if text == "\u{3}" || (control && matches!(text.as_str(), "c" | "C")))
    });
    Some(selection.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, Modifiers};
    use std::borrow::Cow;

    fn key_c(modifiers: Modifiers, pressed: bool) -> Event {
        Event::Key {
            key: eframe::egui::Key::C,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn selected_terminal_path_copies_with_control_command_or_native_copy() {
        let path = "src/a folder/日本.rs:17";
        for (mut events, modifiers) in [
            (vec![key_c(Modifiers::CTRL, true)], Modifiers::CTRL),
            (vec![key_c(Modifiers::MAC_CMD, true)], Modifiers::MAC_CMD),
            (vec![Event::Copy], Modifiers::NONE),
            (vec![Event::Text("\u{3}".into())], Modifiers::NONE),
        ] {
            assert_eq!(
                take_terminal_copy(&mut events, modifiers, path).as_deref(),
                Some(path)
            );
            assert!(events.is_empty());
        }
    }

    #[test]
    fn copying_a_selection_consumes_duplicate_control_text_without_interrupting() {
        let paste = Event::Paste("other".into());
        let mut events = vec![
            key_c(Modifiers::CTRL, true),
            Event::Text("\u{3}".into()),
            Event::Text("c".into()),
            Event::Copy,
            paste.clone(),
        ];
        assert_eq!(
            take_terminal_copy(&mut events, Modifiers::CTRL, "path"),
            Some("path".into())
        );
        assert_eq!(events, [paste]);
        assert_eq!(
            take_terminal_copy(&mut events, Modifiers::CTRL, "path"),
            None
        );
    }

    #[test]
    fn no_selection_leaves_control_c_and_native_copy_for_the_focused_widget() {
        let original = vec![
            key_c(Modifiers::CTRL, true),
            Event::Text("\u{3}".into()),
            Event::Copy,
        ];
        let mut events = original.clone();
        assert_eq!(take_terminal_copy(&mut events, Modifiers::CTRL, ""), None);
        assert_eq!(events, original);
    }

    #[test]
    fn selection_does_not_consume_ordinary_text_other_chords_or_key_release() {
        for (event, modifiers) in [
            (Event::Text("c".into()), Modifiers::NONE),
            (
                key_c(Modifiers::CTRL | Modifiers::ALT, true),
                Modifiers::CTRL | Modifiers::ALT,
            ),
            (key_c(Modifiers::CTRL, false), Modifiers::CTRL),
            (Event::Paste("path".into()), Modifiers::NONE),
        ] {
            let mut events = vec![event.clone()];
            assert_eq!(take_terminal_copy(&mut events, modifiers, "selected"), None);
            assert_eq!(events, [event]);
        }
    }

    #[test]
    fn image_paste_retains_unique_private_pngs_with_exact_pixels() {
        #[cfg(unix)]
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().join("clipboard ' space");
        std::fs::create_dir(&dir).unwrap();
        let pixels = [255, 0, 10, 255, 0, 255, 20, 128];
        let image = arboard::ImageData {
            width: 2,
            height: 1,
            bytes: Cow::Borrowed(&pixels),
        };
        let first = save_image(&image, &dir).unwrap();
        let second = save_image(&image, &dir).unwrap();
        assert_ne!(first, second);
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            assert!(
                first == terminator_core::quote(&path.to_string_lossy())
                    || second == terminator_core::quote(&path.to_string_lossy())
            );
            #[cfg(unix)]
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(image::open(path).unwrap().into_rgba8().as_raw(), &pixels);
        }
    }

    #[test]
    fn malformed_image_does_not_leave_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let image = arboard::ImageData {
            width: usize::MAX,
            height: 2,
            bytes: Cow::Borrowed(&[]),
        };
        assert!(save_image(&image, dir.path()).is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn image_intent_is_consumed_once_and_text_is_untouched() {
        let mut events = vec![Event::Paste(String::new()), Event::Paste("hello".into())];
        let modifiers = Modifiers::COMMAND | Modifiers::SHIFT;
        assert_eq!(take_image_paste(&mut events, modifiers), 1);
        assert_eq!(take_image_paste(&mut events, modifiers), 0);
        assert_eq!(events, vec![Event::Paste("hello".into())]);
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn plain_control_v_is_left_for_the_terminal() {
        let mut events = vec![Event::Paste(String::new())];
        assert_eq!(take_image_paste(&mut events, Modifiers::CTRL), 0);
        assert_eq!(events.len(), 1);
    }
}
