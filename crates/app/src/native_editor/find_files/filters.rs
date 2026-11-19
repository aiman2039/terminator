//! The Find in Files filters: the File mask / Exclude row folds behind a
//! toggle on the query row; folded, the set filters show as a short chip
//! beside the query. Whether the row shows is GUI memory only, kept
//! across opens of the window.
use super::*;

/// The longest filter summary beside the query.
const FILTER_CHIP_CHARS: usize = 28;

fn filters_open_id() -> egui::Id {
    egui::Id::unique("find-in-files-filters-open")
}

/// Whether the File mask / Exclude row shows.
pub(super) fn filters_open(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<bool>(filters_open_id()))
        .unwrap_or(false)
}

pub(super) fn set_filters_open(ctx: &egui::Context, open: bool) {
    ctx.data_mut(|data| data.insert_temp(filters_open_id(), open));
}

/// The set filters in short, for the folded row: `*.rs · −target/**`.
pub(super) fn filter_summary(mask: &str, exclude: &str) -> Option<String> {
    let mut parts = Vec::new();
    if !mask.trim().is_empty() {
        parts.push(mask.trim().to_owned());
    }
    if !exclude.trim().is_empty() {
        parts.push(format!("−{}", exclude.trim()));
    }
    if parts.is_empty() {
        return None;
    }
    let text = parts.join(" · ");
    Some(if text.chars().count() > FILTER_CHIP_CHARS {
        let cut: String = text
            .chars()
            .take(FILTER_CHIP_CHARS.saturating_sub(1))
            .collect();
        format!("{cut}…")
    } else {
        text
    })
}

/// The chip's text while the row is folded and a filter is set.
pub(super) fn folded_summary(ctx: &egui::Context, find: &FindInFiles) -> Option<String> {
    if filters_open(ctx) {
        None
    } else {
        filter_summary(&find.mask, &find.exclude)
    }
}

/// The room the chip takes on the query row.
pub(super) fn chip_width(ui: &egui::Ui, summary: Option<&String>) -> f32 {
    summary.map_or(0.0, |text| {
        let galley = ui.painter().layout_no_wrap(
            text.clone(),
            egui::FontId::proportional(appearance::FONT_CAPTION),
            ui.visuals().text_color(),
        );
        galley.size().x + 4.0 * ui.spacing().item_spacing.x
    })
}

/// The chip beside the query; a click opens the row.
pub(super) fn filter_chip(ui: &mut egui::Ui, find: &FindInFiles, summary: &str) {
    let response = appearance::link(
        ui,
        egui::RichText::new(summary).size(appearance::FONT_CAPTION),
    )
    .on_hover_text(format!(
        "File mask: {}\nExclude: {}\nClick to edit",
        find.mask.trim(),
        find.exclude.trim()
    ));
    #[cfg(feature = "test-support")]
    diagnostics::record(ui.ctx(), "find-files-filter-chip", response.rect);
    if response.clicked() {
        set_filters_open(ui.ctx(), true);
    }
}

/// The File mask and Exclude globs, under the query row while open.
pub(super) fn filter_row(ui: &mut egui::Ui, find: &mut FindInFiles, keys: egui::EventFilter) {
    ui.horizontal(|ui| {
        let width = ((ui.available_width() - 130.0) / 2.0).max(80.0);
        let glob = |ui: &mut egui::Ui, text: &mut String, name: &str, hint: &str| {
            let field = ui.add(
                appearance::singleline(text)
                    .id(super::view::field_id(name))
                    .event_filter(keys)
                    .hint_text(hint)
                    .desired_width(width),
            );
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), &format!("find-files-{name}"), field.rect);
            #[cfg(not(feature = "test-support"))]
            let _ = field;
        };
        ui.weak("File mask");
        glob(ui, &mut find.mask, "mask", "*.rs, src/**");
        ui.weak("Exclude");
        glob(ui, &mut find.exclude, "exclude", "target/**, *.min.js");
    });
}

/// The filter toggle, last on the query row.
pub(super) fn filter_toggle(ui: &mut egui::Ui) {
    let open = filters_open(ui.ctx());
    let toggle = appearance::selectable_icon(appearance::SelectableIconInput {
        ui,
        icon: "Funnel",
        tip: "File mask and Exclude",
        selected: open,
    });
    #[cfg(feature = "test-support")]
    diagnostics::record(ui.ctx(), "find-files-filters", toggle.rect);
    if toggle.clicked() {
        set_filters_open(ui.ctx(), !open);
    }
}
