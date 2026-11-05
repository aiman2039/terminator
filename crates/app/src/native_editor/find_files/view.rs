//! The Find in Files window: query and toggles, the replacement in
//! replace mode, file mask and Exclude, the scope and status, results
//! grouped by file (with include checkboxes in replace mode), and the
//! preview. Keys work while one of its fields has focus: Up and Down move
//! the cursor, Enter opens the hit at its match, Esc closes. A click
//! picks a hit; a double click opens it.
use super::cursor::{Row, keep_visible};
use super::preview::{JobColors, line_job, preview_pane};
use super::replace::{DiffColors, ReplaceAsk, changes_pane, replace_buttons};
use super::*;

const TITLE: &str = "Find in Files";
/// List row height, the same for file headers and hits (`show_rows`).
const ROW_HEIGHT: f32 = 22.0;
/// Visible height of the results list.
const LIST_HEIGHT: f32 = 264.0;
/// The window's text fields; the keys above act while one has focus.
const FIELDS: [&str; 4] = ["query", "replace", "mask", "exclude"];
/// The fields keep Escape and the arrows, so focus stays put.
const FIELD_KEYS: egui::EventFilter = egui::EventFilter {
    tab: false,
    horizontal_arrows: true,
    vertical_arrows: true,
    escape: true,
};

fn field_id(name: &str) -> egui::Id {
    egui::Id::unique(("find-in-files", name))
}

/// What the window asks for once it is painted.
enum Outcome {
    Open(NavLocation),
    Close,
    Replace(ReplaceAsk),
}

/// Window keys taken this frame.
#[derive(Default)]
struct Keys {
    up: usize,
    down: usize,
    enter: bool,
    escape: bool,
}

impl App {
    pub(crate) fn show_find_in_files_window(&mut self, ctx: &egui::Context) {
        self.drive_replace(ctx);
        self.drive_find_in_files();
        let colors = DiffColors {
            added: appearance::color(&self.theme.git_added),
            removed: appearance::color(&self.theme.status_failed),
        };
        let Some(find) = self.find_in_files.as_mut().filter(|find| find.shown) else {
            return;
        };
        let mut open = true;
        let mut outcome = None;
        self.popups
            .window(ctx, TITLE)
            .open(&mut open)
            .collapsible(false)
            .default_width(720.0)
            .show(ctx, |ui| outcome = find_body(ui, find, colors));
        if find.running {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        let closed = !open || matches!(outcome, Some(Outcome::Close));
        if closed && let Some(id) = find.return_focus.take() {
            ctx.memory_mut(|memory| memory.request_focus(id));
        }
        if closed || matches!(outcome, Some(Outcome::Open(_))) {
            find.hide();
        }
        match outcome {
            Some(Outcome::Open(target)) => {
                self.record_jump_origin();
                self.open_location(&target);
            }
            Some(Outcome::Replace(ask)) => self.start_replace(ask == ReplaceAsk::SelectedFile),
            Some(Outcome::Close) | None => {}
        }
    }
}

fn find_body(ui: &mut egui::Ui, find: &mut FindInFiles, colors: DiffColors) -> Option<Outcome> {
    let keys = take_keys(ui);
    if keys.escape {
        return Some(Outcome::Close);
    }
    for _ in 0..keys.up {
        find.step(false);
    }
    for _ in 0..keys.down {
        find.step(true);
    }
    // A replace in flight keeps the input as the replace saw it.
    let open = ui
        .add_enabled_ui(!find.replace.busy(), |ui| {
            find_inputs(ui, find);
            if find.replace.on {
                replace_input(ui, find);
            }
            scope_row(ui, find);
            results(ui, find)
        })
        .inner;
    let ask = if find.replace.on {
        changes_pane(ui, find, colors);
        replace_buttons(ui, find, colors)
    } else {
        preview_pane(ui, find);
        None
    };
    if let Some(ask) = ask {
        return Some(Outcome::Replace(ask));
    }
    if keys.enter || open {
        return find.selected_location().map(Outcome::Open);
    }
    None
}

/// Arrows, Enter, and Escape, taken before the fields see them, while
/// one of the window's fields has focus.
fn take_keys(ui: &mut egui::Ui) -> Keys {
    let focused = ui.memory(|memory| memory.focused());
    if !FIELDS.iter().any(|name| focused == Some(field_id(name))) {
        return Keys::default();
    }
    let none = egui::Modifiers::NONE;
    ui.input_mut(|input| Keys {
        up: input.count_and_consume_key(none, egui::Key::ArrowUp),
        down: input.count_and_consume_key(none, egui::Key::ArrowDown),
        enter: input.consume_key(none, egui::Key::Enter),
        escape: input.consume_key(none, egui::Key::Escape),
    })
}

/// Width of a text field beside the four toggle icons.
fn field_width(ui: &egui::Ui) -> f32 {
    (ui.available_width() - 120.0).max(160.0)
}

/// Query row with toggles (replace mode last), then the file mask and
/// Exclude globs.
fn find_inputs(ui: &mut egui::Ui, find: &mut FindInFiles) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let mut output = appearance::singleline(&mut find.query)
            .id(field_id("query"))
            .event_filter(FIELD_KEYS)
            .hint_text("Text to find")
            .desired_width(field_width(ui))
            .show(ui);
        if std::mem::take(&mut find.select_query) {
            let all = egui::text::CCursorRange::select_all(&output.galley);
            output.state.cursor.set_char_range(Some(all));
            output.state.store(ui.ctx(), output.response.id);
            // Opening: remember where the keyboard was, for Esc.
            let before = ui.memory(|memory| memory.focused());
            if before.is_some_and(|id| !FIELDS.iter().any(|name| field_id(name) == id)) {
                find.return_focus = before;
            }
        }
        if std::mem::take(&mut find.focus) {
            output.response.request_focus();
        }
        for (flag, icon, tip) in [
            (&mut find.case, "CaseSensitive", "Match case"),
            (&mut find.word, "WholeWord", "Words"),
            (&mut find.regex, "Regex", "Regex"),
        ] {
            if appearance::selectable_icon(appearance::SelectableIconInput {
                ui,
                icon,
                tip,
                selected: *flag,
            })
            .clicked()
            {
                *flag = !*flag;
            }
        }
        if appearance::selectable_icon(appearance::SelectableIconInput {
            ui,
            icon: "Pencil",
            tip: "Replace",
            selected: find.replace.on,
        })
        .clicked()
        {
            find.set_replace_mode(!find.replace.on);
        }
    });
    ui.horizontal(|ui| {
        let width = ((ui.available_width() - 130.0) / 2.0).max(80.0);
        ui.weak("File mask");
        glob_field(ui, (&mut find.mask, "mask"), "*.rs, src/**", width);
        ui.weak("Exclude");
        glob_field(
            ui,
            (&mut find.exclude, "exclude"),
            "target/**, *.min.js",
            width,
        );
    });
}

/// The replacement, under the query; `$1` inserts group 1 in regex mode.
fn replace_input(ui: &mut egui::Ui, find: &mut FindInFiles) {
    ui.horizontal(|ui| {
        let hint = if find.regex {
            "Replace with ($1 for groups)"
        } else {
            "Replace with"
        };
        let mut output = appearance::singleline(&mut find.replace.text)
            .id(field_id("replace"))
            .event_filter(FIELD_KEYS)
            .hint_text(hint)
            .desired_width(field_width(ui))
            .show(ui);
        if std::mem::take(&mut find.replace.select) {
            let all = egui::text::CCursorRange::select_all(&output.galley);
            output.state.cursor.set_char_range(Some(all));
            output.state.store(ui.ctx(), output.response.id);
        }
        if std::mem::take(&mut find.replace.focus) {
            output.response.request_focus();
        }
    });
}

fn glob_field(ui: &mut egui::Ui, (text, name): (&mut String, &str), hint: &str, width: f32) {
    ui.add(
        appearance::singleline(text)
            .id(field_id(name))
            .event_filter(FIELD_KEYS)
            .hint_text(hint)
            .desired_width(width),
    );
}

/// Where the search runs (an X widens a folder to the project), then the
/// status on the right.
fn scope_row(ui: &mut egui::Ui, find: &mut FindInFiles) {
    ui.horizontal(|ui| {
        ui.weak("In");
        let scope = find.folder.as_deref().map_or_else(
            || {
                find.root.file_name().map_or_else(
                    || find.root.display().to_string(),
                    |name| name.to_string_lossy().into_owned(),
                )
            },
            |folder| crate::workspace_ops::relative_display(&find.root, folder),
        );
        ui.label(scope)
            .on_hover_text(find.walk_folder().display().to_string());
        if find.folder.is_some()
            && appearance::sidebar_action(ui, "X", "Search the whole project").clicked()
        {
            find.folder = None;
            find.focus = true;
        }
        ui.with_layout(
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| match &find.error {
                Some(error) => {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                None => {
                    ui.weak(summary(find));
                }
            },
        );
    });
}

/// The status: progress, the counts, or the refine hint at the limit.
pub(super) fn summary(find: &FindInFiles) -> String {
    if find.query.trim().is_empty() {
        return "Type to search".into();
    }
    if find.truncated {
        return format!("{}+ results, refine the search", find.total);
    }
    let files = find.files.len();
    let counts = format!(
        "{} match{} in {} file{}",
        find.total,
        if find.total == 1 { "" } else { "es" },
        files,
        if files == 1 { "" } else { "s" },
    );
    match (find.running, find.total) {
        (true, 0) => "Searching…".into(),
        (true, _) => format!("Searching… {counts}"),
        (false, 0) => "No matches".into(),
        (false, _) => counts,
    }
}

/// What one list row asks for: pick a hit, open it, or flip the
/// include checkbox of a file (`None` line) or one line.
#[derive(Default)]
struct RowAsk {
    pick: Option<usize>,
    open: bool,
    toggle: Option<(PathBuf, Option<u32>)>,
}

/// The grouped hits, painted only while visible. A click moves the
/// cursor; a double click asks to open. Returns that ask.
fn results(ui: &mut egui::Ui, find: &mut FindInFiles) -> bool {
    let mut area = egui::ScrollArea::vertical()
        .id_salt("find-in-files-results")
        .max_height(LIST_HEIGHT)
        .auto_shrink([false, false]);
    if std::mem::take(&mut find.reveal) {
        let top = find
            .row_of(find.selected)
            .map_or(0.0, |row| row as f32 * ROW_HEIGHT);
        area = area.vertical_scroll_offset(keep_visible(top, ROW_HEIGHT, LIST_HEIGHT, find.scroll));
    }
    let mut asks = Vec::new();
    let output = ui
        .scope(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            // Checkboxes keep the row height.
            ui.spacing_mut().interact_size.y = ROW_HEIGHT;
            area.show_rows(ui, ROW_HEIGHT, find.row_count(), |ui, range| {
                for row in find.rows(range) {
                    asks.push(list_row(ui, find, row));
                }
            })
        })
        .inner;
    find.scroll = output.state.clamped_offset().y;
    let mut open = false;
    for ask in asks {
        if let Some(index) = ask.pick {
            find.select(index);
            // Keep the keys working: focus goes back to the field.
            find.refocus();
        }
        if let Some((path, line)) = ask.toggle {
            find.toggle_include(&path, line);
            find.refocus();
        }
        open |= ask.open;
    }
    open
}

/// One list row; in replace mode its include checkbox comes first.
fn list_row(ui: &mut egui::Ui, find: &FindInFiles, row: Row<'_>) -> RowAsk {
    let replacing = find.replace.on;
    let mut ask = RowAsk::default();
    match row {
        Row::File(file) => {
            let check = replacing.then(|| find.replace.file_check(file));
            let (toggled, ()) = with_check(ui, check, |ui| file_row(ui, &find.root, file));
            ask.toggle = toggled.then(|| (file.path.clone(), None));
        }
        Row::Hit { file, hit, index } => {
            let included = !replacing || find.replace.line_included(&file.path, hit.line);
            let check = replacing.then_some((included, false));
            let looks = (index == find.selected, !included);
            let (toggled, response) = with_check(ui, check, |ui| hit_row(ui, hit, looks));
            ask.toggle = toggled.then(|| (file.path.clone(), Some(hit.line)));
            if response.clicked() || response.double_clicked() {
                ask.pick = Some(index);
            }
            ask.open = response.double_clicked();
        }
    }
    ask
}

/// `row` after an include checkbox when `check` is `Some((checked,
/// partial))`. Returns whether the box was clicked, and `row`'s result.
fn with_check<R>(
    ui: &mut egui::Ui,
    check: Option<(bool, bool)>,
    row: impl FnOnce(&mut egui::Ui) -> R,
) -> (bool, R) {
    let Some((checked, partial)) = check else {
        return (false, row(ui));
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let mut value = checked;
        let check = egui::Checkbox::without_text(&mut value).indeterminate(partial);
        let clicked = ui.add(check).clicked();
        (clicked, row(ui))
    })
    .inner
}

/// A file header row: path below the root, then its hit count.
fn file_row(ui: &mut egui::Ui, root: &Path, file: &FileHits) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_HEIGHT),
        egui::Sense::hover(),
    );
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter_at(rect);
    let name = painter.layout_no_wrap(
        crate::workspace_ops::relative_display(root, &file.path),
        egui::FontId::proportional(appearance::FONT_BODY),
        ui.visuals().strong_text_color(),
    );
    let top = rect.center().y - name.size().y * 0.5;
    let after = rect.left() + 4.0 + name.size().x + 6.0;
    painter.galley(
        egui::pos2(rect.left() + 4.0, top),
        name,
        ui.visuals().text_color(),
    );
    painter.text(
        egui::pos2(after, rect.center().y),
        egui::Align2::LEFT_CENTER,
        format!("({})", file.hits.len()),
        egui::FontId::proportional(appearance::FONT_SMALL),
        ui.visuals().weak_text_color(),
    );
}

/// One hit: muted line number, then the line with its matches marked.
/// A line a replace leaves out (`dim`) shows weak.
fn hit_row(ui: &mut egui::Ui, hit: &LineHit, (selected, dim): (bool, bool)) -> egui::Response {
    let response = appearance::popup_row(ui, ROW_HEIGHT, selected);
    if !ui.is_rect_visible(response.rect) {
        return response;
    }
    let rect = response.rect;
    let painter = ui.painter_at(rect);
    painter.text(
        egui::pos2(rect.left() + 52.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        hit.line.saturating_add(1).to_string(),
        egui::FontId::monospace(appearance::FONT_SMALL),
        ui.visuals().weak_text_color(),
    );
    let left = rect.left() + 62.0;
    let colors = JobColors {
        plain: if dim {
            ui.visuals().weak_text_color()
        } else {
            ui.visuals().text_color()
        },
        found: appearance::editor_style().palette.find_match,
    };
    let mut job = line_job(&hit.preview, &[], &hit.matches, colors);
    job.wrap.max_width = (rect.right() - 6.0 - left).max(0.0);
    let galley = ui.painter().layout_job(job);
    painter.galley(
        egui::pos2(left, rect.center().y - galley.size().y * 0.5),
        galley,
        ui.visuals().text_color(),
    );
    response
}
