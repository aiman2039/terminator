//! Editor navigation (`IntelliJ`): Back/Forward through jump origins, a
//! chooser when a declaration has several targets, and the Find Usages
//! window (grouped by file, with line previews, open until closed).

use super::*;
use crate::app::OpenFileInput;
use std::collections::HashMap;
use terminator_native_edit::highlight::{Span, highlight_text, language_for_path};

/// Jumps remembered for Back; older ones fall off.
const HISTORY_LIMIT: usize = 64;
/// Usages shown per search; previews are read for at most these files.
const USAGE_LIMIT: usize = 500;

/// A caret position to return to: file plus LSP (UTF-16) position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NavLocation {
    pub path: PathBuf,
    pub line: u32,
    pub character: u32,
}

/// The Find Usages (or declaration chooser) window.
pub(crate) struct UsagesView {
    pub title: &'static str,
    /// Usages grouped by file, in server order, each `(line, character)`.
    pub groups: Vec<(PathBuf, Vec<(u32, u32)>)>,
    pub total: usize,
    /// Line text per `(file, line)`: open buffers fill in at once, other
    /// files through a background read.
    pub previews: HashMap<(PathBuf, u32), String>,
    /// Syntax spans per previewed line, filled the first time it is drawn.
    pub highlighted: HashMap<(PathBuf, u32), Vec<Span>>,
    pub generation: u64,
}

impl App {
    /// Where the caret is in the active native editor, if any.
    pub(crate) fn current_location(&self) -> Option<NavLocation> {
        self.location_of(self.active_editor_path_ref()?)
    }

    /// The caret of the open buffer for `path`.
    pub(crate) fn location_of(&self, path: &Path) -> Option<NavLocation> {
        let (line, character) = self.native_docs.get(path)?.caret_position()?;
        Some(NavLocation {
            path: path.to_owned(),
            line,
            character,
        })
    }

    /// Remember where a jump starts, so Back returns there. A new jump
    /// forgets the forward trail, as in a browser.
    pub(crate) fn record_jump_origin(&mut self) {
        let Some(here) = self.current_location() else {
            return;
        };
        if self.nav_back.last() != Some(&here) {
            self.nav_back.push(here);
            if self.nav_back.len() > HISTORY_LIMIT {
                self.nav_back.remove(0);
            }
        }
        self.nav_forward.clear();
    }

    /// Back (`forward == false`) or Forward through the jump history.
    pub(crate) fn navigate_history(&mut self, forward: bool) {
        let target = if forward {
            self.nav_forward.pop()
        } else {
            self.nav_back.pop()
        };
        let Some(target) = target else {
            return;
        };
        if let Some(here) = self.current_location() {
            if forward {
                self.nav_back.push(here);
            } else {
                self.nav_forward.push(here);
            }
        }
        self.open_location(&target);
    }

    pub(crate) fn open_location(&mut self, target: &NavLocation) {
        let position = native_lsp::Position {
            line: target.line,
            character: target.character,
        };
        let absolute = std::path::absolute(&target.path).unwrap_or_else(|_| target.path.clone());
        self.open_file(OpenFileInput {
            path: target.path.clone(),
            line: position.line.checked_add(1),
            split: None,
            external: false,
        });
        if self.native_pending_line.contains_key(&absolute) {
            self.native_pending_col.insert(absolute, position.character);
        }
    }

    /// Show locations in the usages window, reading previews for files
    /// that are not open in the background.
    pub(crate) fn show_usages(
        &mut self,
        title: &'static str,
        targets: Vec<native_lsp::GotoTarget>,
    ) {
        let mut groups: Vec<(PathBuf, Vec<(u32, u32)>)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut total = 0usize;
        for target in targets.into_iter().take(USAGE_LIMIT) {
            let Some(path) = native_lsp::uri_to_path(&target.uri) else {
                continue;
            };
            let at = (target.range.start.line, target.range.start.character);
            if !seen.insert((path.clone(), at)) {
                continue;
            }
            total = total.saturating_add(1);
            match groups.iter_mut().find(|(file, _)| *file == path) {
                Some((_, items)) => items.push(at),
                None => groups.push((path, vec![at])),
            }
        }
        for (_, items) in &mut groups {
            items.sort_unstable();
        }
        let generation = self
            .usages
            .as_ref()
            .map_or(1, |view| view.generation.wrapping_add(1));
        let mut previews = HashMap::new();
        let mut missing: Vec<(PathBuf, Vec<u32>)> = Vec::new();
        for (path, items) in &groups {
            let lines: Vec<u32> = items.iter().map(|(line, _)| *line).collect();
            match self.native_docs.get(path).and_then(|doc| doc.buffer()) {
                Some(buffer) => {
                    for line in lines {
                        let index = usize::try_from(line).unwrap_or(usize::MAX);
                        previews.insert((path.clone(), line), buffer.line_text(index));
                    }
                }
                None => missing.push((path.clone(), lines)),
            }
        }
        if !missing.is_empty() {
            let _ = self.jobs.send(Job::LinePreviews(generation, missing));
        }
        self.usages = Some(UsagesView {
            title,
            groups,
            total,
            previews,
            highlighted: HashMap::new(),
            generation,
        });
    }

    /// Previews read off the GUI thread for the current usages search.
    pub(crate) fn apply_line_previews(
        &mut self,
        generation: u64,
        lines: Vec<((PathBuf, u32), String)>,
    ) {
        if let Some(view) = self.usages.as_mut()
            && view.generation == generation
        {
            view.previews.extend(lines);
        }
    }

    /// The usages window, styled like the `IntelliJ` Find Usages popup:
    /// a muted summary, files with counts, and each usage as a muted line
    /// number plus its syntax-colored line. Clicking jumps; the window
    /// stays open (the declaration chooser closes on a pick).
    pub(crate) fn show_usages_window(&mut self, ctx: &egui::Context) {
        let root = self.selected_project().map(|project| project.path.clone());
        let Some(view) = self.usages.as_mut() else {
            return;
        };
        let title = view.title;
        let chooser = title == DECLARATION_TITLE;
        let mut open = true;
        let mut picked: Option<NavLocation> = None;
        self.popups
            .window(ctx, title)
            .open(&mut open)
            .collapsible(false)
            .default_width(560.0)
            .frame(appearance::popup_frame(&ctx.global_style()).inner_margin(10))
            .show(ctx, |ui| {
                appearance::popup_text_style(ui.style_mut());
                picked = usages_body(ui, view, root.as_deref());
            });
        if let Some(target) = picked {
            self.record_jump_origin();
            self.open_location(&target);
            if chooser {
                open = false;
            }
        }
        if !open {
            self.usages = None;
        }
    }
}

/// Summary line and the grouped usage rows; returns a clicked usage.
fn usages_body(
    ui: &mut egui::Ui,
    view: &mut UsagesView,
    root: Option<&std::path::Path>,
) -> Option<NavLocation> {
    if view.total == 0 {
        ui.weak("No usages found.");
        return None;
    }
    ui.weak(format!(
        "{} usage{} in {} file{}",
        view.total,
        if view.total == 1 { "" } else { "s" },
        view.groups.len(),
        if view.groups.len() == 1 { "" } else { "s" },
    ));
    ui.add_space(4.0);
    let mut picked = None;
    let UsagesView {
        groups,
        previews,
        highlighted,
        ..
    } = view;
    egui::ScrollArea::vertical()
        .max_height(420.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (path, items) in groups.iter() {
                let shown = root
                    .and_then(|root| path.strip_prefix(root).ok())
                    .unwrap_or(path);
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(shown.display().to_string()).strong());
                    ui.weak(format!("({})", items.len()));
                });
                ui.add_space(2.0);
                for (line, character) in items {
                    let key = (path.clone(), *line);
                    let preview = previews.get(&key).map_or("", |text| text.trim());
                    let spans = highlighted
                        .entry(key)
                        .or_insert_with(|| preview_spans(path, preview, ui.visuals().dark_mode));
                    if usage_row(ui, line.saturating_add(1), preview, spans).clicked() {
                        picked = Some(NavLocation {
                            path: path.clone(),
                            line: *line,
                            character: *character,
                        });
                    }
                }
            }
        });
    picked
}

/// Syntax spans for one trimmed preview line (empty when not highlighted).
fn preview_spans(path: &std::path::Path, text: &str, dark: bool) -> Vec<Span> {
    if text.is_empty() {
        return Vec::new();
    }
    let language = language_for_path(&path.to_string_lossy());
    highlight_text(language, text, dark)
        .and_then(|lines| lines.into_iter().next())
        .unwrap_or_default()
}

/// The line text as a layout job: span colors over the plain text color.
pub(crate) fn preview_job(
    text: &str,
    spans: &[Span],
    plain: egui::Color32,
) -> egui::text::LayoutJob {
    let font = egui::FontId::monospace(12.5);
    let mut job = egui::text::LayoutJob::default();
    let chars: Vec<char> = text.chars().collect();
    let mut col = 0usize;
    let push = |job: &mut egui::text::LayoutJob, from: usize, to: usize, color| {
        let piece: String = chars.get(from..to).unwrap_or_default().iter().collect();
        if !piece.is_empty() {
            job.append(&piece, 0.0, egui::TextFormat::simple(font.clone(), color));
        }
    };
    for span in spans {
        let start = span.start.min(chars.len());
        let end = span.end.min(chars.len());
        if start > col {
            push(&mut job, col, start, plain);
        }
        if end > start.max(col) {
            let [r, g, b] = span.color;
            push(
                &mut job,
                start.max(col),
                end,
                egui::Color32::from_rgb(r, g, b),
            );
            col = end;
        }
    }
    push(&mut job, col, chars.len(), plain);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job
}

/// One usage: muted right-aligned line number, then the colored line.
fn usage_row(ui: &mut egui::Ui, number: u32, preview: &str, spans: &[Span]) -> egui::Response {
    let response = appearance::popup_row(ui, 24.0, false);
    if !ui.is_rect_visible(response.rect) {
        return response;
    }
    let rect = response.rect;
    let painter = ui.painter_at(rect);
    let muted = ui.visuals().weak_text_color();
    painter.text(
        egui::pos2(rect.left() + 44.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        number.to_string(),
        egui::FontId::monospace(12.0),
        muted,
    );
    let text_left = rect.left() + 58.0;
    let mut job = preview_job(preview, spans, ui.visuals().text_color());
    job.wrap.max_width = (rect.right() - 6.0 - text_left).max(0.0);
    let galley = ui.painter().layout_job(job);
    painter.galley(
        egui::pos2(text_left, rect.center().y - galley.size().y * 0.5),
        galley,
        ui.visuals().text_color(),
    );
    response
}

/// Window title of the declaration chooser (several definition targets).
pub(crate) const DECLARATION_TITLE: &str = "Choose declaration";

/// Read the requested lines of each file (worker side). Files over 4 MiB
/// or unreadable are skipped: a preview is a convenience.
pub(crate) fn read_line_previews(files: Vec<(PathBuf, Vec<u32>)>) -> Vec<((PathBuf, u32), String)> {
    const MAX_BYTES: u64 = 4 * 1024 * 1024;
    let mut out = Vec::new();
    for (path, lines) in files {
        if std::fs::metadata(&path).is_ok_and(|meta| meta.len() > MAX_BYTES) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let all: Vec<&str> = text.lines().collect();
        for line in lines {
            if let Some(content) = usize::try_from(line).ok().and_then(|index| all.get(index)) {
                out.push(((path.clone(), line), (*content).to_owned()));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_returns_to_the_jump_origin_and_forward_comes_back() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
        app.state.settings.editor_mode = EditorMode::Native;
        app.state.projects.push(Project {
            id: "p".into(),
            name: "p".into(),
            path: dir.path().into(),
            layout: serde_json::Value::Null,
        });
        app.selected = Some("p".into());
        let path = dir.path().join("a.rs");
        app.layouts.insert(
            "p".into(),
            Workspace::from_layout(egui_dock::DockState::new(vec![Tab::NativeEditor {
                path: path.clone(),
            }])),
        );
        let mut native = NativeDoc::new(path.clone(), true);
        let mut buffer = Doc::new("one\ntwo\nthree\n");
        let _ = native.engine.place_cursor(&mut buffer, 2, 1);
        native.doc = Some(buffer);
        app.native_docs.insert(path.clone(), native);

        app.record_jump_origin();
        let origin = NavLocation {
            path: path.clone(),
            line: 2,
            character: 1,
        };
        assert_eq!(app.nav_back, std::slice::from_ref(&origin));
        // Recording the same place twice keeps one entry.
        app.record_jump_origin();
        assert_eq!(app.nav_back.len(), 1);

        app.navigate_history(false);
        assert!(app.nav_back.is_empty());
        assert_eq!(app.nav_forward, std::slice::from_ref(&origin));
        assert_eq!(app.native_pending_line.get(&path), Some(&2));
        assert_eq!(app.native_pending_col.get(&path), Some(&1));
        app.navigate_history(true);
        assert_eq!(app.nav_back, [origin]);
        assert!(app.nav_forward.is_empty());
    }

    #[test]
    fn several_definitions_open_the_chooser_grouped_by_file() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
        let target = |name: &str, line: u32| native_lsp::GotoTarget {
            uri: native_lsp::file_uri(&dir.path().join(name))
                .map(|uri| uri.to_string())
                .unwrap_or_default(),
            range: native_lsp::Range {
                start: native_lsp::Position { line, character: 0 },
                end: native_lsp::Position { line, character: 0 },
            },
        };
        app.apply_goto_jump(lsp_manager::GotoJump {
            from: dir.path().join("main.rs"),
            targets: vec![target("a.rs", 4), target("b.rs", 1), target("a.rs", 9)],
        });
        let view = app.usages.as_ref().expect("chooser");
        assert_eq!(view.title, DECLARATION_TITLE);
        assert_eq!(view.total, 3);
        assert_eq!(view.groups.len(), 2);
        assert_eq!(view.groups[0].1, [(4, 0), (9, 0)]);
    }

    #[test]
    fn usage_previews_keep_their_syntax_colors() {
        let text = "pub struct RowSpec<'a> {";
        let spans = preview_spans(std::path::Path::new("list.rs"), text, true);
        assert!(!spans.is_empty(), "rust preview is highlighted");
        let plain = egui::Color32::from_rgb(1, 2, 3);
        let job = preview_job(text, &spans, plain);
        assert_eq!(job.text, text);
        // `pub` takes the Islands Dark keyword color, not the plain one.
        let first = job.sections.first().expect("first section");
        assert_eq!(
            first.format.color,
            egui::Color32::from_rgb(0xCF, 0x8E, 0x6D)
        );
        // Unknown files stay plain and keep every char.
        let job = preview_job("plain words", &[], plain);
        assert_eq!(job.text, "plain words");
        assert!(
            job.sections
                .iter()
                .all(|section| section.format.color == plain)
        );
    }
}
