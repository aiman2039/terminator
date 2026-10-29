//! Saved-host thumbnails written by the daemon (`Paths::vnc_thumbnail`).
//! Files load on a background thread; a newer file replaces the texture.
use super::*;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::mpsc::{Receiver, TryRecvError, channel},
    time::SystemTime,
};

/// How often to look for a newer file per host.
const RECHECK: Duration = Duration::from_secs(10);

type Loaded = Option<(SystemTime, ColorImage)>;

#[derive(Default)]
pub(crate) struct ThumbnailCache {
    entries: HashMap<String, Entry>,
}

#[derive(Default)]
struct Entry {
    texture: Option<TextureHandle>,
    modified: Option<SystemTime>,
    checked: Option<Instant>,
    loading: Option<Receiver<Loaded>>,
}

impl ThumbnailCache {
    /// The host's last saved screen, if any. Starts a background load when
    /// the file was not checked recently.
    pub(crate) fn get(
        &mut self,
        ctx: &egui::Context,
        id: &str,
        path: Option<PathBuf>,
    ) -> Option<TextureHandle> {
        let entry = self.entries.entry(id.to_owned()).or_default();
        if let Some(receiver) = &entry.loading {
            match receiver.try_recv() {
                Ok(Some((modified, image))) => {
                    entry.texture = Some(ctx.load_texture(
                        format!("vnc-thumbnail:{id}"),
                        image,
                        egui::TextureOptions::LINEAR,
                    ));
                    entry.modified = Some(modified);
                    entry.loading = None;
                }
                Ok(None) | Err(TryRecvError::Disconnected) => entry.loading = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        let due = entry.checked.is_none_or(|t| t.elapsed() >= RECHECK);
        if let Some(path) = path.filter(|_| due && entry.loading.is_none()) {
            entry.checked = Some(Instant::now());
            entry.loading = Some(load(ctx.clone(), path, entry.modified));
        }
        entry.texture.clone()
    }
}

/// Read and decode `path` unless it is not newer than `known`.
fn load(ctx: egui::Context, path: PathBuf, known: Option<SystemTime>) -> Receiver<Loaded> {
    let (sender, receiver) = channel();
    let _ = std::thread::Builder::new()
        .name("vnc-thumbnail-load".into())
        .spawn(move || {
            let loaded = std::fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .ok()
                .filter(|modified| Some(*modified) != known)
                .and_then(|modified| {
                    let image = image::open(&path).ok()?.to_rgba8();
                    let size = [
                        usize::try_from(image.width()).ok()?,
                        usize::try_from(image.height()).ok()?,
                    ];
                    let image = ColorImage::from_rgba_unmultiplied(size, image.as_raw());
                    Some((modified, image))
                });
            let changed = loaded.is_some();
            let _ = sender.send(loaded);
            if changed {
                ctx.request_repaint();
            }
        });
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_thumbnail_loads_in_the_background() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p1.png");
        image::RgbaImage::from_pixel(4, 2, image::Rgba([1, 2, 3, 255]))
            .save(&path)
            .unwrap();
        let ctx = egui::Context::default();
        let mut cache = ThumbnailCache::default();
        let mut texture = None;
        for _ in 0..200 {
            texture = cache.get(&ctx, "p1", Some(path.clone()));
            if texture.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(texture.map(|t| t.size()), Some([4, 2]));
        assert!(cache.get(&ctx, "missing", None).is_none());
    }
}
