//! Markdown images use the same bounded background decoder as image tabs.
use eframe::egui::{
    self,
    load::{ImageLoader, ImagePoll, LoadError},
};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

type Decoded = Result<Arc<egui::ColorImage>, String>;
struct Entry {
    ticket: u64,
    result: Option<Decoded>,
    cancel: terminator_core::async_service::CancellationToken,
}
pub struct Images {
    entries: Arc<Mutex<HashMap<String, Entry>>>,
    services: crate::gui_services::Services,
    #[cfg(test)]
    _owner: Option<Mutex<crate::gui_services::Owner>>,
    next: std::sync::atomic::AtomicU64,
}
impl Images {
    #[cfg(test)]
    pub fn install(ctx: &egui::Context) -> Arc<Self> {
        let paths = terminator_core::Paths::at(std::env::temp_dir().join(terminator_core::id()));
        let (tx, _) = std::sync::mpsc::channel();
        let (services, owner) = crate::gui_services::Services::new(paths, ctx.clone(), tx).unwrap();
        let loader = Arc::new(Self {
            entries: Default::default(),
            services,
            next: Default::default(),
            _owner: Some(Mutex::new(owner)),
        });
        ctx.add_image_loader(loader.clone());
        loader
    }
    pub fn with_services(
        ctx: &egui::Context,
        services: crate::gui_services::Services,
    ) -> Arc<Self> {
        let loader = Arc::new(Self {
            entries: Default::default(),
            services,
            next: Default::default(),
            #[cfg(test)]
            _owner: None,
        });
        ctx.add_image_loader(loader.clone());
        loader
    }
    pub fn clear(&self, ctx: &egui::Context) {
        self.retain(ctx, &HashSet::new());
    }
    pub fn retain(&self, ctx: &egui::Context, used: &HashSet<String>) {
        let unused: Vec<_> = self
            .entries
            .lock()
            .unwrap()
            .keys()
            .filter(|uri| !used.contains(*uri))
            .cloned()
            .collect();
        for uri in unused {
            ctx.forget_image(&uri);
        }
    }
}
impl ImageLoader for Images {
    fn id(&self) -> &str {
        concat!(module_path!(), "::Images")
    }
    fn load(
        &self,
        ctx: &egui::Context,
        uri: &str,
        _: egui::SizeHint,
    ) -> egui::load::ImageLoadResult {
        if !uri.starts_with("markdown-image:") {
            return Err(LoadError::NotSupported);
        }
        let mut entries = self.entries.lock().unwrap();
        if let Some(entry) = entries.get(uri) {
            return match &entry.result {
                Some(Ok(image)) => Ok(ImagePoll::Ready {
                    image: image.clone(),
                }),
                Some(Err(error)) => Err(LoadError::Loading(error.clone())),
                None => Ok(ImagePoll::Pending { size: None }),
            };
        }
        if entries.len() >= 32 {
            return Err(LoadError::Loading(
                "Markdown preview supports up to 32 local images".into(),
            ));
        }
        let ticket = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        use terminator_core::async_service::{CancellationToken, OperationContext, Policy};
        let token = CancellationToken::new();
        let cancelled = token.clone();
        let cache = self.entries.clone();
        let service = self.services.clone();
        let target = uri.to_owned();
        let repaint = ctx.clone();
        let context =
            OperationContext::new("markdown-image", target.clone(), Policy::ReplaceableRead);
        let result = self
            .services
            .handle()
            .submit(context, token.clone(), async move {
                let path = target
                    .strip_prefix("markdown-image:")
                    .and_then(|s| url::Url::parse(s).ok())
                    .and_then(|u| u.to_file_path().ok());
                let result = match path {
                    Some(path) => crate::image_preview::load(&service, path, &cancelled)
                        .await
                        .map(Arc::new)
                        .map_err(|e| format!("{e:#}")),
                    None => Err("Invalid local image path".into()),
                };
                let mut entries = cache.lock().unwrap();
                let used: usize = entries
                    .values()
                    .filter_map(|e| e.result.as_ref())
                    .filter_map(|r| r.as_ref().ok())
                    .map(|i| i.pixels.len() * 4)
                    .sum();
                if let Some(entry) = entries.get_mut(&target).filter(|e| e.ticket == ticket) {
                    entry.result = Some(result.and_then(|image| {
                        if used + image.pixels.len() * 4 > 64 * 1024 * 1024 {
                            Err("Markdown images exceed the 64 MiB preview limit".into())
                        } else {
                            Ok(image)
                        }
                    }));
                }
                repaint.request_repaint();
                Ok(Vec::new())
            });
        if result.is_ok() {
            entries.insert(
                uri.into(),
                Entry {
                    ticket,
                    result: None,
                    cancel: token,
                },
            );
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        Ok(ImagePoll::Pending { size: None })
    }
    fn forget(&self, uri: &str) {
        self.entries.lock().unwrap().remove(uri);
    }
    fn forget_all(&self) {
        self.entries.lock().unwrap().clear();
    }
    fn byte_size(&self) -> usize {
        self.entries
            .lock()
            .unwrap()
            .values()
            .filter_map(|v| v.result.as_ref())
            .filter_map(|r| r.as_ref().ok())
            .map(|i| i.pixels.len() * 4)
            .sum()
    }
    fn has_pending(&self) -> bool {
        self.entries
            .lock()
            .unwrap()
            .values()
            .any(|v| v.result.is_none())
    }
}

impl Drop for Entry {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_images_decode_off_thread_and_can_be_reloaded_and_released() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("picture.png");
        image::RgbaImage::from_pixel(12, 8, image::Rgba([210, 60, 40, 255]))
            .save(&path)
            .unwrap();
        let ctx = egui::Context::default();
        let images = Images::install(&ctx);
        let uri = format!(
            "markdown-image:{}",
            url::Url::from_file_path(&path).unwrap()
        );
        let wait = || {
            let start = std::time::Instant::now();
            loop {
                match images.load(&ctx, &uri, egui::SizeHint::default()) {
                    Ok(ImagePoll::Pending { .. }) => {
                        assert!(start.elapsed() < std::time::Duration::from_secs(3));
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    result => return result,
                }
            }
        };
        let ImagePoll::Ready { image } = wait().unwrap() else {
            panic!("Image not decoded")
        };
        assert_eq!(image.size, [12, 8]);
        assert_eq!(images.byte_size(), 12 * 8 * 4);
        std::fs::write(&path, "broken image").unwrap();
        images.clear(&ctx);
        assert!(matches!(wait(), Err(LoadError::Loading(_))));
        assert!(matches!(
            images.load(&ctx, "https://example.com/a.png", egui::SizeHint::default()),
            Err(LoadError::NotSupported)
        ));
        images.clear(&ctx);
        assert_eq!(images.byte_size(), 0);
    }
}
