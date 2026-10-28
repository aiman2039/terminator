//! Editor, browser, and image tab results.

use std::path::PathBuf;

use crate::*;

/// Most texture memory image previews may hold at once.
const PREVIEW_MEMORY_LIMIT: usize = 128 * 1024 * 1024;

pub(super) struct ImageInput {
    pub(super) path: PathBuf,
    pub(super) generation: u64,
    pub(super) result: Result<egui::ColorImage, String>,
}

impl App {
    pub(super) fn handle_resolved_target(&mut self, key: String, target: Option<services::Target>) {
        self.loading.remove(&key);
        if self.targets.len() > 256 {
            self.targets.clear();
        }
        self.targets.insert(key, target);
    }

    pub(super) fn handle_text(&mut self, key: String, text: String) {
        self.loading.remove(&key);
        self.texts.insert(key, text);
    }

    pub(super) fn handle_open_native_editor(
        &mut self,
        project: String,
        path: PathBuf,
        after: After,
    ) {
        let existing = self.layouts.get(&project).and_then(|workspace| {
            workspace
                .tabs
                .iter()
                .flat_map(|group| group.layout.iter_all_tabs())
                .map(|(_, tab)| tab)
                .find(|tab| matches!(tab, Tab::NativeEditor { path: current } if *current == path))
                .cloned()
        });
        self.open_gui_tab(
            project,
            existing.unwrap_or(Tab::NativeEditor { path }),
            after,
        );
    }

    pub(super) fn handle_open_browser(
        &mut self,
        project: String,
        target: BrowserTarget,
        after: After,
    ) {
        let existing = self.layouts.get(&project).and_then(|workspace| {
            workspace
                .tabs
                .iter()
                .flat_map(|group| group.layout.iter_all_tabs())
                .map(|(_, tab)| tab)
                .find(
                    |tab| matches!(tab, Tab::Browser { target: current, .. } if *current == target),
                )
                .cloned()
        });
        self.open_gui_tab(
            project,
            existing.unwrap_or_else(|| Tab::Browser { id: id(), target }),
            after,
        );
    }

    pub(super) fn handle_image(&mut self, ctx: &egui::Context, input: ImageInput) {
        let ImageInput {
            path,
            generation,
            result,
        } = input;
        let used: usize = self
            .images
            .values()
            .filter_map(|p| p.texture.as_ref())
            .map(Self::texture_bytes)
            .sum();
        let Some(preview) = self
            .images
            .get_mut(&path)
            .filter(|p| p.generation == generation)
        else {
            return;
        };
        preview.loading = false;
        let previous_bytes = preview.texture.as_ref().map_or(0, Self::texture_bytes);
        match result {
            Ok(image)
                if used
                    .saturating_sub(previous_bytes)
                    .saturating_add(image.pixels.len().saturating_mul(4))
                    <= PREVIEW_MEMORY_LIMIT =>
            {
                preview.texture = Some(ctx.load_texture(
                    format!("preview:{}:{generation}", path.display()),
                    image,
                    egui::TextureOptions::LINEAR,
                ));
            }
            Ok(_) => {
                preview.error =
                    Some("Preview memory limit reached; close another image and retry.".into());
            }
            Err(error) => preview.error = Some(error),
        }
    }

    fn texture_bytes(texture: &egui::TextureHandle) -> usize {
        texture.size()[0]
            .saturating_mul(texture.size()[1])
            .saturating_mul(4)
    }
}
