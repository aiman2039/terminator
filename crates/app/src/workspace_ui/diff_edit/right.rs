use super::super::super::*;
use diff::live::EditorMarks;
use terminator_native_edit::highlight::Language;
use terminator_native_edit::view::{ShowSourceInput, SourceOptions, show_source};
use terminator_native_edit::vim::Effect;

type PaneOutput = egui::scroll_area::ScrollAreaOutput<()>;

/// What the diff's editor pane needs beyond the buffer.
pub(crate) struct DiffEditorOptionsInput<'a> {
    pub(crate) vim: bool,
    pub(crate) read_only: bool,
    pub(crate) marks: &'a EditorMarks,
    pub(crate) sync: f32,
}

/// Editor options for the diff's right pane: the file editor's font,
/// palette and menus, one font row per line (the painted left pane draws
/// text at the row top), no soft wrap or sticky headers (both would break
/// row alignment), and the shared scroll offset.
pub(crate) fn diff_editor_options(input: DiffEditorOptionsInput<'_>) -> SourceOptions {
    let DiffEditorOptionsInput {
        vim,
        read_only,
        marks,
        sync,
    } = input;
    let style = appearance::editor_style();
    SourceOptions {
        vim,
        read_only,
        menu_style: Some(appearance::menu_style),
        menu_header: Some(appearance::target_header),
        menu_item: Some(|ui, label, icon, shortcut| {
            appearance::menu_item(appearance::MenuItemInput {
                ui,
                label,
                icon,
                shortcut,
            })
        }),
        font_size: style.prefs.editor_font_size,
        line_height: 1.0,
        caret_blink: style.prefs.editor_caret_blink,
        palette: Some(style.palette),
        // The status row carries the Vim command line; modeless needs none.
        show_status: vim,
        autofocus: false,
        soft_wrap: false,
        sticky_headers: false,
        scroll_offset: Some(sync),
        spacers: std::sync::Arc::clone(&marks.spacers),
        line_fills: std::sync::Arc::clone(&marks.line_fills),
        range_fills: std::sync::Arc::clone(&marks.range_fills),
        ..SourceOptions::default()
    }
}

pub(crate) struct PaintRightEditorInput<'a> {
    pub(crate) ui: &'a mut egui::Ui,
    pub(crate) rect: egui::Rect,
    pub(crate) buffer: &'a mut diff::review::Edit,
    pub(crate) options: &'a SourceOptions,
    pub(crate) language: Language,
    pub(crate) key: &'a str,
}

/// The editor pane's scroll state (shaped like the painted panes', so the
/// connectors, chevrons and jumps work unchanged) and whether the editor
/// asked to save (`:w`, Cmd+S).
pub(crate) struct RightEditorOutput {
    pub(crate) pane: PaneOutput,
    pub(crate) save: bool,
}

pub(in super::super) fn paint_right_editor(input: PaintRightEditorInput<'_>) -> RightEditorOutput {
    let PaintRightEditorInput {
        ui,
        rect,
        buffer,
        options,
        language,
        key,
    } = input;
    let id = format!("{key}#diff-editor");
    let editor = &mut buffer.editor;
    let outcome = ui
        .scope_builder(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| {
                ui.set_clip_rect(rect.intersect(ui.clip_rect()));
                show_source(ShowSourceInput {
                    ui,
                    doc: &mut editor.doc,
                    engine: &mut editor.engine,
                    options,
                    highlight: &mut editor.highlight,
                    language,
                    id: &id,
                })
            },
        )
        .inner;
    buffer.sync_text(ui.input(|input| input.time));
    #[cfg(feature = "test-support")]
    diagnostics::record(
        ui.ctx(),
        "diff-right-editor",
        outcome.content_rect.intersect(outcome.scroll.inner_rect),
    );
    let save = outcome.effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::Save | Effect::SaveForce | Effect::WriteQuit { .. }
        )
    });
    RightEditorOutput {
        pane: outcome.scroll,
        save,
    }
}
