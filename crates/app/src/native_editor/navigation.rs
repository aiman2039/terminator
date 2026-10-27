//! Editor navigation (`IntelliJ`): Back/Forward through jump origins, a
//! chooser when a declaration has several targets, and the Find Usages
//! window (grouped by file, with line previews, open until closed).

use super::*;
use std::collections::HashMap;

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
    pub generation: u64,
}

impl App {
    /// Where the caret is in the active native editor, if any.
    pub(crate) fn current_location(&self) -> Option<NavLocation> {
        let path = self.active_editor_path()?;
        let doc = self.native_docs.get(&path)?;
        let (line, character) = doc.caret_position()?;
        Some(NavLocation {
            path,
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
        self.open_file(
            target.path.clone(),
            position.line.checked_add(1),
            None,
            false,
        );
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

    /// The usages window: files with counts, each usage as `line:col`
    /// plus its trimmed line. Clicking jumps; the window stays open (the
    /// declaration chooser closes on a pick).
    pub(crate) fn show_usages_window(&mut self, ctx: &egui::Context) {
        let Some(view) = self.usages.as_ref() else {
            return;
        };
        let title = view.title;
        let chooser = title == DECLARATION_TITLE;
        let mut open = true;
        let mut picked: Option<NavLocation> = None;
        let root = self.selected_project().map(|project| project.path.clone());
        self.popups
            .window(ctx, title)
            .open(&mut open)
            .collapsible(false)
            .default_width(520.0)
            .show(ctx, |ui| {
                if view.total == 0 {
                    ui.weak("No usages found.");
                    return;
                }
                ui.weak(format!(
                    "{} usage{} in {} file{}",
                    view.total,
                    if view.total == 1 { "" } else { "s" },
                    view.groups.len(),
                    if view.groups.len() == 1 { "" } else { "s" },
                ));
                egui::ScrollArea::vertical()
                    .max_height(420.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for (path, items) in &view.groups {
                            let shown = root
                                .as_deref()
                                .and_then(|root| path.strip_prefix(root).ok())
                                .unwrap_or(path);
                            ui.horizontal(|ui| {
                                ui.strong(shown.display().to_string());
                                ui.weak(format!("({})", items.len()));
                            });
                            for (line, character) in items {
                                let preview = view
                                    .previews
                                    .get(&(path.clone(), *line))
                                    .map(|text| text.trim().to_owned())
                                    .unwrap_or_default();
                                let label = egui::RichText::new(format!(
                                    "{:>5}  {preview}",
                                    line.saturating_add(1)
                                ))
                                .monospace();
                                if ui
                                    .add(egui::Button::selectable(false, label).truncate())
                                    .clicked()
                                {
                                    picked = Some(NavLocation {
                                        path: path.clone(),
                                        line: *line,
                                        character: *character,
                                    });
                                }
                            }
                            ui.add_space(4.0);
                        }
                    });
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
}
