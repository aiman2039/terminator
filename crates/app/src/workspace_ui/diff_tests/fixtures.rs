use super::*;

pub(super) fn span(text: &str) -> diff::DiffSpan {
    diff::DiffSpan {
        text: text.into(),
        rgb: [209, 211, 217],
        intra: diff::Intra::None,
    }
}

pub(super) struct LineInput<'a> {
    pub(super) kind: diff::LineKind,
    pub(super) old_no: Option<u32>,
    pub(super) new_no: Option<u32>,
    pub(super) text: &'a str,
}

pub(super) fn line(input: LineInput<'_>) -> diff::DiffLine {
    let LineInput {
        kind,
        old_no,
        new_no,
        text,
    } = input;
    diff::DiffLine {
        kind,
        old_no,
        new_no,
        spans: vec![span(text)],
    }
}

pub(super) fn sample_line() -> diff::DiffLine {
    line(LineInput {
        kind: diff::LineKind::Insert,
        old_no: None,
        new_no: Some(1),
        text: "visible-diff-marker",
    })
}

pub(super) fn sample_doc() -> diff::DiffDocument {
    diff::DiffDocument {
        left_label: "Index".into(),
        right_label: "Working tree".into(),
        left_text: String::new(),
        right_text: "visible-diff-marker\n".into(),
        unified: vec![sample_line()],
        split: vec![],
        ..Default::default()
    }
}

pub(super) fn paint_diff_view(
    app: &mut App,
    ctx: &egui::Context,
    tab: &Tab,
) -> Vec<(egui::Pos2, String)> {
    let mut painted = Vec::new();
    for _ in 0..2 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 500.0),
                )),
                ..Default::default()
            },
            |ui| {
                // egui_dock tab body: no pane scrollbars, expand to the leaf,
                // then the native viewer owns its own ScrollArea.
                egui::ScrollArea::new([false, false]).show(ui, |ui| {
                    let available = ui.available_rect_before_wrap();
                    ui.expand_to_include_rect(available);
                    app.diff_view(ui, tab);
                });
            },
        );
        painted = painted_text(&output.shapes);
        output.textures_delta.clear();
    }
    painted
}

pub(super) fn paint_doc(doc: diff::DiffDocument, split: bool) -> Vec<(egui::Pos2, String)> {
    paint_doc_staged(doc, split, false)
}

pub(super) fn paint_doc_staged(
    doc: diff::DiffDocument,
    split: bool,
    staged: bool,
) -> Vec<(egui::Pos2, String)> {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    let tab = Tab::Diff {
        remote: None,
        cwd: "/repo".into(),
        path: "/repo/file.rs".into(),
        staged,
    };
    if split {
        app.diff_split.insert(tab.key());
    }
    app.diffs.insert(tab.key(), Ok(doc.into()));
    paint_diff_view(&mut app, &ctx, &tab)
}

pub(super) fn require_text<'a>(
    painted: &'a [(egui::Pos2, String)],
    needle: &str,
) -> &'a (egui::Pos2, String) {
    painted
        .iter()
        .find(|(_, text)| text == needle || text.lines().any(|line| line.trim_end() == needle))
        .unwrap_or_else(|| panic!("{needle} was not painted: {painted:?}"))
}

pub(super) fn scroll_doc(
    ctx: &egui::Context,
    doc: &diff::DiffDocument,
    split: bool,
) -> (
    Vec<egui::scroll_area::ScrollAreaOutput<()>>,
    Vec<egui::epaint::ClippedShape>,
) {
    let mut scroll = None;
    let mut ratio = 0.5;
    let mut sync = 0.0;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 300.0),
            )),
            ..Default::default()
        },
        |ui| {
            scroll = Some(paint_diff_document(PaintDiffDocumentInput {
                ui,
                doc,
                split,
                colors: DiffColors {
                    added: Color32::GREEN,
                    deleted: Color32::RED,
                    accent: Color32::BLUE,
                    text: Color32::WHITE,
                },
                scroll_key: "scroll-regression",
                ratio: &mut ratio,
                sync: &mut sync,
            }));
        },
    );
    output.textures_delta.clear();
    (scroll.unwrap(), output.shapes)
}

pub(crate) fn long_doc() -> diff::DiffDocument {
    let unified: Vec<_> = (1..=150)
        .map(|n| {
            line(LineInput {
                kind: diff::LineKind::Equal,
                old_no: Some(n),
                new_no: Some(n),
                text: &if n == 1 {
                    format!("{}END", "long-line-".repeat(30))
                } else {
                    format!("short-{n}")
                },
            })
        })
        .collect();
    let split = unified
        .iter()
        .map(|line| diff::SplitRow {
            left: Some(line.clone()),
            right: Some(line.clone()),
        })
        .collect();
    diff::DiffDocument {
        left_label: "old".into(),
        right_label: "new".into(),
        left_text: String::new(),
        right_text: String::new(),
        unified,
        split,
        ..Default::default()
    }
}
