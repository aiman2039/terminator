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
    let mut count = 0;
    events.retain(|event| {
        if matches!(event, eframe::egui::Event::Paste(text) if text.is_empty()) {
            count += 1;
            false
        } else {
            true
        }
    });
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, Modifiers};
    use std::borrow::Cow;

    #[test]
    fn image_paste_retains_unique_private_pngs_with_exact_pixels() {
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
