//! Saves a small PNG of each saved host's screen while it is connected, so
//! the address book can show the last screen after the session ends.
use super::*;
use std::{
    path::{Path, PathBuf},
    sync::Weak,
};

/// Widest thumbnail; height keeps the screen's aspect ratio.
const MAX_WIDTH: usize = 320;
const FIRST_SAVE: Duration = Duration::from_secs(3);
const SAVE_EVERY: Duration = Duration::from_secs(30);

/// Save `runtime`'s screen to `path` shortly after it connects and then
/// every [`SAVE_EVERY`], until the session is gone.
pub(super) fn spawn(runtime: Weak<Mutex<VncRuntime>>, path: PathBuf) {
    let _ = thread::Builder::new()
        .name("vnc-thumbnail".into())
        .spawn(move || {
            thread::sleep(FIRST_SAVE);
            while let Some(runtime) = runtime.upgrade() {
                let shot = {
                    let rt = relock(&runtime);
                    if rt.status == VncSessionStatus::Connected {
                        downscale(&rt)
                    } else {
                        None
                    }
                };
                drop(runtime);
                if let Some((width, height, pixels)) = shot {
                    let _ = save(&path, width, height, pixels);
                }
                thread::sleep(SAVE_EVERY);
            }
        });
}

/// Nearest-neighbour copy of the framebuffer at most [`MAX_WIDTH`] wide.
fn downscale(rt: &VncRuntime) -> Option<(u32, u32, Vec<u8>)> {
    let (src_w, src_h) = (usize::from(rt.width), usize::from(rt.height));
    if src_w == 0 || src_h == 0 || rt.rgba.len() < src_w.saturating_mul(src_h).saturating_mul(4) {
        return None;
    }
    let width = src_w.min(MAX_WIDTH);
    let height = src_h.saturating_mul(width).checked_div(src_w)?.max(1);
    let mut pixels = Vec::with_capacity(width.saturating_mul(height).saturating_mul(4));
    for y in 0..height {
        let sy = y.saturating_mul(src_h).checked_div(height)?;
        for x in 0..width {
            let sx = x.saturating_mul(src_w).checked_div(width)?;
            let at = sy
                .saturating_mul(src_w)
                .saturating_add(sx)
                .saturating_mul(4);
            pixels.extend_from_slice(rt.rgba.get(at..at.saturating_add(4))?);
        }
    }
    Some((
        u32::try_from(width).ok()?,
        u32::try_from(height).ok()?,
        pixels,
    ))
}

/// Write the PNG next to its final name, then rename, so a reader never
/// sees a partial file. The directory is private to the user.
fn save(path: &Path, width: u32, height: u32, pixels: Vec<u8>) -> Result<()> {
    let dir = path.parent().context("Thumbnail path has no directory")?;
    ensure!(
        !fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_symlink()),
        "Thumbnail directory is a symlink"
    );
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    fs::set_permissions(dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
    let image =
        image::RgbaImage::from_raw(width, height, pixels).context("Thumbnail size mismatch")?;
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png)?;
    let partial = path.with_extension("png.partial");
    fs::write(&partial, png.into_inner())?;
    fs::rename(&partial, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wide_screen_is_saved_small_with_its_aspect_ratio() {
        let (tx, _rx) = mpsc::channel(1);
        let mut rt = VncRuntime::new(false, tx);
        rt.width = 1280;
        rt.height = 720;
        rt.rgba = vec![200; 1280 * 720 * 4];
        let (width, height, pixels) = downscale(&rt).unwrap();
        assert_eq!((width, height), (320, 180));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vnc-thumbnails").join("p1.png");
        save(&path, width, height, pixels).unwrap();
        let back = image::open(&path).unwrap();
        assert_eq!((back.width(), back.height()), (320, 180));
        assert!(!path.with_extension("png.partial").exists());
    }
}
