//! Merge conflict resolver (`IntelliJ` Resolve Conflicts, simplified): the
//! conflicted file's marker blocks side by side, Accept Ours / Theirs /
//! Both per block, then save and mark the file resolved (stage it).
//! File reads and writes run on the file pool, never the GUI thread.

use crate::{App, appearance, workspace_ops};
use eframe::egui;
use std::path::{Path, PathBuf};
use terminator_core::async_service::{CancellationToken, OperationContext, Policy};

/// One `<<<<<<<` … `>>>>>>>` block, by line index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConflictBlock {
    pub start: usize,
    pub end: usize,
    pub ours_label: String,
    pub theirs_label: String,
    pub ours: Vec<String>,
    /// The common ancestor (`diff3` / `zdiff3` conflict style).
    pub base: Option<Vec<String>>,
    pub theirs: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Choice {
    Ours,
    Theirs,
    Both,
}

/// Conflict blocks in `text`. Unterminated blocks are ignored.
pub(crate) fn parse_conflicts(text: &str) -> Vec<ConflictBlock> {
    let lines: Vec<&str> = text.lines().collect();
    let mut blocks = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let Some(head) = lines
            .get(index)
            .and_then(|line| line.strip_prefix("<<<<<<<"))
        else {
            index = index.saturating_add(1);
            continue;
        };
        let start = index;
        let mut ours = Vec::new();
        let mut base: Option<Vec<String>> = None;
        let mut theirs = Vec::new();
        // 0: ours, 1: base, 2: theirs.
        let mut part = 0u8;
        let mut cursor = start.saturating_add(1);
        let mut closed = None;
        while let Some(line) = lines.get(cursor) {
            if line.starts_with("|||||||") && part == 0 {
                part = 1;
                base = Some(Vec::new());
            } else if line.starts_with("=======") && part < 2 {
                part = 2;
            } else if let Some(tail) = line.strip_prefix(">>>>>>>").filter(|_| part == 2) {
                closed = Some((cursor, tail.trim().to_owned()));
                break;
            } else {
                match part {
                    0 => ours.push((*line).to_owned()),
                    1 => base.get_or_insert_with(Vec::new).push((*line).to_owned()),
                    _ => theirs.push((*line).to_owned()),
                }
            }
            cursor = cursor.saturating_add(1);
        }
        let Some((end, theirs_label)) = closed else {
            break;
        };
        blocks.push(ConflictBlock {
            start,
            end,
            ours_label: head.trim().to_owned(),
            theirs_label,
            ours,
            base,
            theirs,
        });
        index = end.saturating_add(1);
    }
    blocks
}

/// `text` with one block replaced by the chosen side(s).
pub(crate) fn resolve(text: &str, block: &ConflictBlock, choice: Choice) -> String {
    let chosen: Vec<&String> = match choice {
        Choice::Ours => block.ours.iter().collect(),
        Choice::Theirs => block.theirs.iter().collect(),
        Choice::Both => block.ours.iter().chain(&block.theirs).collect(),
    };
    let ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut out = String::with_capacity(text.len());
    for (index, line) in text.lines().enumerate() {
        if index == block.start {
            for kept in &chosen {
                out.push_str(kept);
                out.push_str(ending);
            }
        }
        if (block.start..=block.end).contains(&index) {
            continue;
        }
        out.push_str(line);
        out.push_str(ending);
    }
    if !text.ends_with('\n') && out.ends_with(ending) {
        out.truncate(out.len().saturating_sub(ending.len()));
    }
    out
}

type FileResult = tokio::sync::mpsc::UnboundedReceiver<Result<String, String>>;

/// Resolver window state for one file.
pub(crate) struct MergeWindow {
    pub path: PathBuf,
    /// Working copy being resolved (`None` until the read lands).
    pub text: Option<String>,
    pub blocks: Vec<ConflictBlock>,
    pub dirty: bool,
    pub error: Option<String>,
    io: Option<FileResult>,
    /// Stage the file once the pending save lands.
    stage_after_save: bool,
}

impl App {
    /// Open the resolver for a conflicted file (read in the background).
    pub(crate) fn open_merge(&mut self, path: PathBuf) {
        let reading = path.clone();
        let io = self.file_io(&path, move || {
            std::fs::read_to_string(&reading)
                .map_err(|error| format!("Read {}: {error}", reading.display()))
        });
        self.merge = Some(MergeWindow {
            path,
            text: None,
            blocks: Vec::new(),
            dirty: false,
            error: None,
            io,
            stage_after_save: false,
        });
    }

    fn file_io(
        &mut self,
        path: &Path,
        work: impl FnOnce() -> Result<String, String> + Send + 'static,
    ) -> Option<FileResult> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let service = self.services.clone();
        let submitted = self.services.handle().submit(
            OperationContext::new(
                "merge-file",
                path.display().to_string(),
                Policy::OrderedMutation,
            ),
            CancellationToken::new(),
            async move {
                let token = CancellationToken::new();
                let result = service
                    .fs()
                    .run(&token, move || Ok(work()))
                    .await
                    .unwrap_or_else(|error| Err(format!("{error:#}")));
                let _ = tx.send(result);
                Ok(Vec::new())
            },
        );
        match submitted {
            Ok(_) => Some(rx),
            Err(error) => {
                self.error = Some(format!("Could not start the file operation: {error}"));
                None
            }
        }
    }

    fn save_merge(&mut self, stage: bool) {
        let Some(window) = self.merge.as_ref() else {
            return;
        };
        let (path, text) = (window.path.clone(), window.text.clone().unwrap_or_default());
        let writing = path.clone();
        let io = self.file_io(&path, move || {
            std::fs::write(&writing, &text)
                .map(|()| text)
                .map_err(|error| format!("Write {}: {error}", writing.display()))
        });
        if let Some(window) = self.merge.as_mut() {
            window.io = io;
            window.stage_after_save = stage;
        }
    }

    pub(crate) fn show_merge_window(&mut self, ctx: &egui::Context) {
        let Some(window) = self.merge.as_mut() else {
            return;
        };
        if let Some(rx) = window.io.as_mut()
            && let Ok(result) = rx.try_recv()
        {
            window.io = None;
            match result {
                Ok(text) => {
                    window.blocks = parse_conflicts(&text);
                    window.text = Some(text);
                    window.dirty = false;
                    window.error = None;
                    if std::mem::take(&mut window.stage_after_save) {
                        let path = window.path.clone();
                        self.queue_workspace(workspace_ops::Op::Stage(path));
                        self.merge = None;
                        self.info = Some("Conflicts resolved and staged".into());
                        return;
                    }
                }
                Err(error) => window.error = Some(error),
            }
        }
        let Some(window) = self.merge.as_mut() else {
            return;
        };
        let title = format!(
            "Resolve conflicts · {}",
            window
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        );
        let mut open = true;
        let mut pick: Option<(usize, Choice)> = None;
        let mut save: Option<bool> = None;
        let busy = window.io.is_some();
        let colors = (
            appearance::color(&self.theme.git_added),
            appearance::color(&self.theme.git_modified),
        );
        self.popups
            .window(ctx, &title)
            .open(&mut open)
            .collapsible(false)
            .default_width(820.0)
            .show(ctx, |ui| {
                if let Some(error) = &window.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                if window.text.is_none() {
                    ui.spinner();
                    return;
                }
                ui.weak(format!(
                    "{} conflict{} left",
                    window.blocks.len(),
                    if window.blocks.len() == 1 { "" } else { "s" }
                ));
                egui::ScrollArea::vertical()
                    .max_height(520.0)
                    .show(ui, |ui| {
                        for (index, block) in window.blocks.iter().enumerate() {
                            ui.separator();
                            ui.horizontal(|ui| {
                                ui.strong(format!(
                                    "Conflict {} · lines {}–{}",
                                    index.saturating_add(1),
                                    block.start.saturating_add(1),
                                    block.end.saturating_add(1)
                                ));
                                ui.add_enabled_ui(!busy, |ui| {
                                    for (label, choice) in [
                                        ("Accept ours", Choice::Ours),
                                        ("Accept theirs", Choice::Theirs),
                                        ("Accept both", Choice::Both),
                                    ] {
                                        if ui.button(label).clicked() {
                                            pick = Some((index, choice));
                                        }
                                    }
                                });
                            });
                            ui.columns(2, |columns| {
                                for (column, (label, lines, color)) in columns.iter_mut().zip([
                                    (
                                        format!("Ours · {}", block.ours_label),
                                        &block.ours,
                                        colors.0,
                                    ),
                                    (
                                        format!("Theirs · {}", block.theirs_label),
                                        &block.theirs,
                                        colors.1,
                                    ),
                                ]) {
                                    column.weak(label);
                                    for line in lines {
                                        column.label(
                                            egui::RichText::new(line).monospace().color(color),
                                        );
                                    }
                                }
                            });
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(!busy, |ui| {
                        if ui
                            .add_enabled(window.dirty, egui::Button::new("Save"))
                            .clicked()
                        {
                            save = Some(false);
                        }
                        if ui
                            .add_enabled(
                                window.blocks.is_empty(),
                                egui::Button::new("Save and mark resolved"),
                            )
                            .on_disabled_hover_text("Resolve every conflict first")
                            .clicked()
                        {
                            save = Some(true);
                        }
                    });
                });
            });
        if let Some((index, choice)) = pick
            && let Some(window) = self.merge.as_mut()
            && let (Some(text), Some(block)) = (window.text.as_ref(), window.blocks.get(index))
        {
            let resolved = resolve(text, block, choice);
            window.blocks = parse_conflicts(&resolved);
            window.text = Some(resolved);
            window.dirty = true;
        }
        if let Some(stage) = save {
            self.save_merge(stage);
        }
        if !open {
            self.merge = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "keep\n<<<<<<< HEAD\nours one\nours two\n||||||| base\nbase\n=======\ntheirs\n>>>>>>> feature\nafter\n";

    #[test]
    fn conflicts_parse_with_labels_and_diff3_base() {
        let blocks = parse_conflicts(TEXT);
        assert_eq!(blocks.len(), 1);
        let block = &blocks[0];
        assert_eq!((block.start, block.end), (1, 8));
        assert_eq!(block.ours_label, "HEAD");
        assert_eq!(block.theirs_label, "feature");
        assert_eq!(block.ours, ["ours one", "ours two"]);
        assert_eq!(block.base.as_deref(), Some(&["base".to_owned()][..]));
        assert_eq!(block.theirs, ["theirs"]);
        assert!(parse_conflicts("<<<<<<< HEAD\nno end\n").is_empty());
    }

    #[test]
    fn resolving_replaces_only_the_block() {
        let block = &parse_conflicts(TEXT)[0];
        assert_eq!(
            resolve(TEXT, block, Choice::Ours),
            "keep\nours one\nours two\nafter\n"
        );
        assert_eq!(
            resolve(TEXT, block, Choice::Theirs),
            "keep\ntheirs\nafter\n"
        );
        assert_eq!(
            resolve(TEXT, block, Choice::Both),
            "keep\nours one\nours two\ntheirs\nafter\n"
        );
        let crlf = TEXT.replace('\n', "\r\n");
        let block = &parse_conflicts(&crlf)[0];
        assert_eq!(
            resolve(&crlf, block, Choice::Theirs),
            "keep\r\ntheirs\r\nafter\r\n"
        );
    }

    #[test]
    fn resolving_saves_and_stages_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, terminator_core::Paths::at(dir.path().join("state")));
        let (jobs, received) = std::sync::mpsc::channel();
        app.jobs = jobs.into();
        app.state.projects.push(terminator_core::Project {
            id: "p".into(),
            name: "p".into(),
            path: dir.path().into(),
            layout: serde_json::Value::Null,
        });
        app.selected = Some("p".into());
        let path = dir.path().join("conflicted.txt");
        std::fs::write(&path, TEXT).unwrap();
        app.open_merge(path.clone());
        let pump = |app: &mut App| {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                app.show_merge_window(ui.ctx())
            });
            output.textures_delta.clear();
        };
        let start = std::time::Instant::now();
        while app
            .merge
            .as_ref()
            .is_some_and(|window| window.text.is_none())
        {
            assert!(start.elapsed() < std::time::Duration::from_secs(10), "read");
            pump(&mut app);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let window = app.merge.as_mut().unwrap();
        assert_eq!(window.blocks.len(), 1);
        let resolved = resolve(
            window.text.as_ref().unwrap(),
            &window.blocks[0],
            Choice::Theirs,
        );
        window.blocks = parse_conflicts(&resolved);
        window.text = Some(resolved);
        window.dirty = true;
        app.save_merge(true);
        while app.merge.is_some() {
            assert!(start.elapsed() < std::time::Duration::from_secs(10), "save");
            pump(&mut app);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "keep\ntheirs\nafter\n"
        );
        assert!(received.try_iter().any(|job| matches!(
            job,
            crate::Job::Workspace(_, workspace_ops::Op::Stage(staged)) if staged == path
        )));
    }
}
