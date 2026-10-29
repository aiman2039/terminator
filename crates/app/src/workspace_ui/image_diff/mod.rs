//! Binary image diffs: when a native diff refuses a binary file that the
//! image preview can decode, show old and new side by side with sizes and
//! dimensions, or as a swipe or onion skin. Reads and decodes run on the
//! file pool; other binaries keep the diff's own message.
use super::super::*;
use eframe::egui::{ColorImage, TextureHandle};
use std::path::{Path, PathBuf};

mod paint;
#[cfg(test)]
mod tests;

/// Image bytes read per side, like the image preview's own file limit.
const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Mode {
    #[default]
    SideBySide,
    Swipe,
    Onion,
}

/// One decoded side, before upload. `image` is `None` when the side has
/// no file (added or deleted).
struct Decoded {
    bytes: usize,
    dims: Option<[u32; 2]>,
    image: Option<Result<ColorImage, String>>,
}

/// One uploaded side.
struct Shown {
    bytes: usize,
    dims: Option<[u32; 2]>,
    texture: Option<Result<TextureHandle, String>>,
}

type Loaded = tokio::sync::mpsc::UnboundedReceiver<Result<[Decoded; 2], String>>;

/// State of one image comparison.
#[derive(Default)]
pub(crate) struct ImageDiff {
    pending: Option<Loaded>,
    sides: Option<Result<[Shown; 2], String>>,
    mode: Mode,
    /// Swipe position or onion-skin opacity of the new side, 0..=1.
    mix: f32,
}

/// Whether a native diff error is a binary image this view can show.
pub(crate) fn is_binary_image(path: &Path, error: &str) -> bool {
    let svg = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"));
    !svg && crate::image_preview::supported(path)
        && (error.starts_with("Binary file") || error.contains("exceeds"))
}

fn git_options() -> terminator_core::CommandOptions {
    terminator_core::CommandOptions {
        stdout_limit: MAX_IMAGE_BYTES,
        ..Default::default()
    }
}

/// The blob behind `spec` (`HEAD:path`, `:0:path`), `None` when absent.
fn object(root: &Path, spec: &str) -> Result<Option<Vec<u8>>, String> {
    let Ok(oid) = terminator_git::run_blocking(
        root,
        &["rev-parse", "--verify", "--quiet", spec],
        git_options(),
    ) else {
        return Ok(None);
    };
    let oid = String::from_utf8_lossy(&oid).trim().to_owned();
    terminator_git::run_blocking(root, &["cat-file", "blob", &oid], git_options())
        .map(Some)
        .map_err(|error| format!("{error:#}"))
}

/// Old and new bytes: HEAD to index when `staged`, else index to disk.
fn read_sides(cwd: &Path, path: &Path, staged: bool) -> Result<[Option<Vec<u8>>; 2], String> {
    let top = terminator_git::run_blocking(cwd, &["rev-parse", "--show-toplevel"], git_options())
        .map_err(|error| format!("{error:#}"))?;
    let root = PathBuf::from(String::from_utf8_lossy(&top).trim());
    let relative =
        terminator_git::relative_path(&root, &cwd.join(path)).map_err(|error| error.to_string())?;
    let name = relative.to_string_lossy();
    let index = object(&root, &format!(":0:{name}"))?;
    if staged {
        return Ok([object(&root, &format!("HEAD:{name}"))?, index]);
    }
    let disk = root.join(&relative);
    let new = if disk.exists() {
        let cancel = terminator_core::async_service::CancellationToken::new();
        Some(crate::image_preview::read(&disk, &cancel).map_err(|error| format!("{error:#}"))?)
    } else {
        None
    };
    Ok([index, new])
}

fn decode(bytes: Option<Vec<u8>>) -> Decoded {
    let Some(bytes) = bytes else {
        return Decoded {
            bytes: 0,
            dims: None,
            image: None,
        };
    };
    let dims = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .ok()
        .and_then(|reader| reader.into_dimensions().ok())
        .map(|(width, height)| [width, height]);
    let size = bytes.len();
    Decoded {
        bytes: size,
        dims,
        image: Some(
            crate::image_preview::decode_bytes(bytes, false).map_err(|error| format!("{error:#}")),
        ),
    }
}

fn load(cwd: &Path, path: &Path, staged: bool) -> Result<[Decoded; 2], String> {
    let [old, new] = read_sides(cwd, path, staged)?;
    Ok([decode(old), decode(new)])
}

fn upload(ctx: &egui::Context, name: &str, side: Decoded) -> Shown {
    Shown {
        bytes: side.bytes,
        dims: side.dims,
        texture: side.image.map(|image| {
            image.map(|image| ctx.load_texture(name, image, egui::TextureOptions::LINEAR))
        }),
    }
}

impl App {
    /// Show `tab`'s binary diff as images. Returns false (and draws
    /// nothing) when `error` is not about a decodable image.
    pub(in super::super) fn binary_image_diff(
        &mut self,
        ui: &mut egui::Ui,
        tab: &Tab,
        error: &str,
    ) -> bool {
        let Tab::Diff {
            remote: None,
            cwd,
            path,
            staged,
        } = tab
        else {
            return false;
        };
        if !is_binary_image(path, error) {
            return false;
        }
        let key = tab.key();
        if !self.image_diffs.contains_key(&key) {
            let (cwd, path, staged) = (cwd.clone(), path.clone(), *staged);
            let label = path.display().to_string();
            let pending = self.file_pool_job(label, move || load(&cwd, &path, staged));
            self.image_diffs.insert(
                key.clone(),
                ImageDiff {
                    pending,
                    mix: 0.5,
                    ..ImageDiff::default()
                },
            );
        }
        if let Some(diff) = self.image_diffs.get_mut(&key) {
            diff.poll(ui.ctx(), &key);
            diff.show(ui);
        }
        true
    }

    /// Offer the merge tool beside a message that refuses a conflicted file.
    pub(in super::super) fn offer_merge(&mut self, ui: &mut egui::Ui, tab: &Tab, message: &str) {
        let Tab::Diff {
            remote: None,
            cwd,
            path,
            ..
        } = tab
        else {
            return;
        };
        if message.contains("merge conflict")
            && ui
                .button("Resolve…")
                .on_hover_text("Open the three-way merge tool")
                .clicked()
        {
            self.open_merge(cwd.join(path));
        }
    }
}

impl ImageDiff {
    fn poll(&mut self, ctx: &egui::Context, key: &str) {
        let Some(rx) = self.pending.as_mut() else {
            return;
        };
        let Ok(result) = rx.try_recv() else {
            return;
        };
        self.pending = None;
        self.sides = Some(result.map(|[old, new]| {
            [
                upload(ctx, &format!("{key}-old"), old),
                upload(ctx, &format!("{key}-new"), new),
            ]
        }));
    }
}
