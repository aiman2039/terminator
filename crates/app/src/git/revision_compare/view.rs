//! The Compare popup in the app: open it, feed it Git answers, paint it,
//! and open the diff that a pick asks for.
use super::TargetKind;
use super::picker::{Choices, ComparePicker, Pick, PickerMode, Row, pickable, rows, stepped};
use crate::{App, Job, appearance, workspace_ops};
use eframe::egui;
use std::path::Path;

/// Shown instead of a Git file action for a file with no local repository.
pub(crate) const REMOTE_NOTE: &str =
    "Git compare, history, and blame need a local file. Use the SSH terminal for remote files.";
const WIDTH: f32 = 420.0;
const ROW_HEIGHT: f32 = 24.0;
const LIST_HEIGHT: f32 = 320.0;

/// Workspace ops whose replies belong to the popup.
pub(crate) fn answers(op: &workspace_ops::Op) -> bool {
    matches!(
        op,
        workspace_ops::Op::CompareTargets
            | workspace_ops::Op::VerifyRevision(_)
            | workspace_ops::Op::FileHistory(_)
    )
}

/// What the popup asked for this frame.
enum Asked {
    Close,
    Pick(Pick),
}

/// Keys the popup takes before its search field sees them.
struct Keys {
    up: bool,
    down: bool,
    enter: bool,
    escape: bool,
}

impl App {
    /// Opens the popup for `path` and starts loading its choices. A remote
    /// file has no local repository: it gets a message instead.
    pub(crate) fn open_compare_picker(&mut self, path: &Path, mode: PickerMode) {
        if self.selected_remote_project().is_some()
            || crate::remote_ui::is_snapshot(&self.paths.data, path)
        {
            self.info = Some(REMOTE_NOTE.into());
            return;
        }
        let Some(picker) = ComparePicker::new(mode, path) else {
            return;
        };
        let _ = self
            .jobs
            .send(Job::Workspace(picker.cwd.clone(), picker.load_op()));
        self.git.picker = Some(picker);
    }

    /// A Git answer for the popup. Answers for a closed popup or another
    /// folder are stale and change nothing.
    pub(crate) fn compare_reply(
        &mut self,
        cwd: &Path,
        op: &workspace_ops::Op,
        result: Result<workspace_ops::Report, String>,
    ) {
        let Some(picker) = self.git.picker.as_mut().filter(|picker| picker.cwd == cwd) else {
            return;
        };
        if let Some(tab) = picker.answer(op, result) {
            self.git.picker = None;
            self.open_git_tab(tab);
        }
    }

    /// Opens the picked diff, or asks Git to check a typed revision first.
    /// Text that Git could read as an option is refused without a call.
    pub(crate) fn compare_pick(&mut self, pick: Pick) {
        let Some(picker) = self.git.picker.as_mut() else {
            return;
        };
        match pick {
            Pick::Open(revision) => {
                let tab = picker.diff_tab(revision);
                self.git.picker = None;
                self.open_git_tab(tab);
            }
            Pick::Verify(_) if picker.checking.is_some() => {}
            Pick::Verify(text) => match super::typed_revision(&text) {
                Ok(rev) => {
                    picker.note = None;
                    picker.checking = Some(rev.to_owned());
                    let op = workspace_ops::Op::VerifyRevision(rev.to_owned());
                    let _ = self.jobs.send(Job::Workspace(picker.cwd.clone(), op));
                }
                Err(note) => picker.note = Some(note),
            },
        }
    }

    /// The popup window, painted after the workspace. Esc, the close
    /// button, or a click outside closes it.
    pub(crate) fn show_compare_picker(&mut self, ctx: &egui::Context) {
        let Some(mut picker) = self.git.picker.take() else {
            return;
        };
        let mut open = true;
        let mut asked = None;
        let shown = self
            .popups
            .window(ctx, picker.title())
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .frame(appearance::popup_frame(&ctx.global_style()).inner_margin(10))
            .show(ctx, |ui| {
                appearance::popup_text_style(ui.style_mut());
                asked = picker_body(ui, &mut picker);
            });
        let outside = shown.is_some_and(|shown| {
            ctx.input(|input| {
                input.pointer.any_pressed()
                    && input
                        .pointer
                        .interact_pos()
                        .is_some_and(|pos| !shown.response.rect.contains(pos))
            })
        });
        if !open || outside || matches!(asked, Some(Asked::Close)) {
            return;
        }
        self.git.picker = Some(picker);
        if let Some(Asked::Pick(pick)) = asked {
            self.compare_pick(pick);
        }
    }
}

/// Search field, list, and footer; returns what the popup asked for.
fn picker_body(ui: &mut egui::Ui, picker: &mut ComparePicker) -> Option<Asked> {
    ui.set_width(WIDTH);
    let keys = ui.input_mut(|input| Keys {
        up: input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
        down: input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
        enter: input.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
        escape: input.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
    });
    if keys.escape {
        return Some(Asked::Close);
    }
    search_field(ui, picker);
    let rows = rows(&picker.choices, &picker.query);
    let choosable = pickable(&rows);
    picker.selected = picker.selected.min(choosable.len().saturating_sub(1));
    let moved = keys.up || keys.down;
    if moved {
        picker.selected = stepped(picker.selected, keys.down, choosable.len());
    }
    let selected = choosable.get(picker.selected).copied();
    let mut asked = None;
    if keys.enter {
        asked = selected
            .and_then(|at| rows.get(at))
            .and_then(Row::pick)
            .map(Asked::Pick);
    }
    match &picker.choices {
        Choices::Loading => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak(match picker.mode {
                    PickerMode::Refs => "Loading branches and tags…",
                    PickerMode::History => "Loading history…",
                });
            });
        }
        Choices::Failed(error) => {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        choices if rows.is_empty() => {
            ui.weak(empty_note(choices));
        }
        _ => {
            if let Some(pick) = picker_list(ui, &rows, selected, moved) {
                asked = Some(Asked::Pick(pick));
            }
        }
    }
    picker_footer(ui, picker.checking.as_deref(), picker.note.as_deref());
    asked
}

/// The file name, then the search field (focused once on open).
fn search_field(ui: &mut egui::Ui, picker: &mut ComparePicker) {
    let name = picker
        .path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (intro, hint) = match picker.mode {
        PickerMode::Refs => (
            format!("Compare {name} with a branch, a tag, or a revision"),
            "Filter, or type a commit or revision",
        ),
        PickerMode::History => (
            format!("Compare {name} with a commit that changed it"),
            "Filter by message, author, or hash",
        ),
    };
    ui.weak(intro);
    let search = ui.add(
        appearance::singleline(&mut picker.query)
            .hint_text(hint)
            .desired_width(f32::INFINITY),
    );
    if std::mem::take(&mut picker.focus) {
        search.request_focus();
    }
    if search.changed() {
        picker.selected = 0;
        picker.note = None;
    }
    ui.add_space(4.0);
}

fn empty_note(choices: &Choices) -> &'static str {
    match choices {
        Choices::Commits(commits) if commits.is_empty() => "No commits changed this file.",
        Choices::Commits(_) => "No commits match.",
        _ => "No branches or tags. Type a commit or revision.",
    }
}

/// The rows, laid out only while visible; returns the pick of a clicked row.
fn picker_list(
    ui: &mut egui::Ui,
    rows: &[Row<'_>],
    selected: Option<usize>,
    moved: bool,
) -> Option<Pick> {
    let mut picked = None;
    let stride = ROW_HEIGHT + ui.spacing().item_spacing.y;
    let mut area = egui::ScrollArea::vertical()
        .max_height(LIST_HEIGHT)
        .auto_shrink([false, true]);
    if moved && let Some(at) = selected {
        let top = f32::from(u16::try_from(at).unwrap_or(u16::MAX)) * stride;
        area = area.vertical_scroll_offset((top - (LIST_HEIGHT - stride) / 2.0).max(0.0));
    }
    area.show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
        for (index, row) in rows.iter().enumerate().take(range.end).skip(range.start) {
            if picker_row(ui, row, selected == Some(index)).clicked() {
                picked = row.pick();
            }
        }
    });
    picked
}

/// One row: a small header, or an icon, a label, and muted details.
fn picker_row(ui: &mut egui::Ui, row: &Row<'_>, selected: bool) -> egui::Response {
    let (icon, label, trailing) = match row {
        Row::Header(text) => {
            let size = egui::vec2(ui.available_width(), ROW_HEIGHT);
            let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
            ui.painter().text(
                egui::pos2(rect.left() + 6.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                *text,
                egui::FontId::proportional(appearance::FONT_CAPTION),
                ui.visuals().weak_text_color(),
            );
            return response;
        }
        Row::Target(target) => (
            match target.kind {
                TargetKind::LocalBranch => "GitBranch",
                TargetKind::RemoteBranch => "Globe",
                TargetKind::Tag => "Bookmark",
            },
            target.name.clone(),
            if target.current { "current branch" } else { "" }.to_owned(),
        ),
        Row::Commit(entry) => (
            "History",
            format!("{}  {}", entry.short_hash, entry.subject),
            format!("{} · {}", entry.author, entry.date),
        ),
        Row::Typed(text) => ("Search", (*text).to_owned(), "check revision".to_owned()),
    };
    let tint = ui.visuals().weak_text_color();
    appearance::row(appearance::RowInput {
        ui,
        label: &label,
        icon,
        selected,
        height: ROW_HEIGHT,
        trailing: &trailing,
        tint,
    })
}

/// Progress or refusal of a typed revision, then the key hints.
fn picker_footer(ui: &mut egui::Ui, checking: Option<&str>, note: Option<&str>) {
    if let Some(text) = checking {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.weak(format!("Checking {text}…"));
        });
    }
    if let Some(note) = note {
        ui.colored_label(ui.visuals().error_fg_color, note);
    }
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new("↑↓ select · ⏎ compare · Esc close")
            .small()
            .weak(),
    );
}
