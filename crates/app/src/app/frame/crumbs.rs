//! The status-bar breadcrumb of the focused native editor. It replaced the
//! editor's path bar (`JetBrains` New UI): a click on a folder or the file
//! reveals it in Explorer, and a click on the caret's symbol opens File
//! Structure. When the row is short of room, the leading folders fold into
//! `…`, then the symbol hides, so the items after the crumbs stay in view.

use crate::*;

/// Padding around one crumb's text.
const CRUMB_PAD: egui::Vec2 = egui::vec2(6.0, 2.0);
/// egui memory: the room the status bar took after the crumbs last frame.
const TRAILING_KEY: &str = "status-crumbs-trailing";
/// egui memory: this frame's pass number and the x where the crumbs ended.
const END_KEY: &str = "status-crumbs-end";

impl App {
    /// `project › src › main.rs › main  Saving…` for the editor at `path`.
    /// Unsaved changes show on the file's tab, not here.
    pub(super) fn status_editor_crumbs(
        &mut self,
        ui: &mut egui::Ui,
        project: &Project,
        path: &std::path::Path,
    ) {
        let crumbs = self.editor_crumbs(project, path);
        let labels: Vec<&str> = crumbs
            .segments
            .iter()
            .map(|segment| segment.label.as_str())
            .collect();
        let texts = CrumbTexts {
            project: &project.name,
            segments: &labels,
            symbol: crumbs.symbol.as_deref(),
            busy: crumbs.busy,
        };
        let fit = CrumbWidths::measure(ui, &texts).fit(crumbs_budget(ui));
        let _project = ui.weak(&project.name);
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "status-breadcrumb-project", _project.rect);
        if let Some(folded) = labels.get(..fit.folded).filter(|folded| !folded.is_empty()) {
            ui.weak("›");
            let _fold = crumb(ui, "…", false).on_hover_text(folded.join(" › "));
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "status-crumb-fold", _fold.rect);
        }
        let mut reveal = None;
        for segment in crumbs.segments.iter().skip(fit.folded) {
            ui.weak("›");
            let response = crumb(ui, &segment.label, segment.reveal.is_some());
            #[cfg(feature = "test-support")]
            {
                let name = format!("status-crumb:{}", segment.label);
                diagnostics::record(ui.ctx(), &name, response.rect);
                diagnostics::record(ui.ctx(), "status-breadcrumb", response.rect);
            }
            if segment.reveal.is_some() && response.on_hover_text("Reveal in Explorer").clicked() {
                reveal.clone_from(&segment.reveal);
            }
        }
        if let Some(target) = reveal
            && !self.reveal_in_explorer(&target)
        {
            self.info = Some(format!("Explorer cannot show {}", target.display()));
        }
        if fit.symbol
            && let Some(symbol) = &crumbs.symbol
        {
            ui.weak("›");
            let response = crumb(ui, symbol, true);
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "status-crumb-symbol", response.rect);
            let keys = self.shortcut_label(Action::FileStructure);
            let tip = appearance::shortcut_tip("File structure", &keys);
            if response.on_hover_text(tip).clicked() {
                self.open_file_structure();
            }
        }
        if let Some(busy) = crumbs.busy {
            ui.weak(busy);
        }
        let end = (ui.ctx().cumulative_pass_nr(), ui.cursor().left());
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::unique(END_KEY), end));
    }

    /// Call after the status-bar row: remember the room the items after
    /// this frame's crumbs took, from the crumbs' end to `flow_end` plus
    /// the right-aligned block, so the next frame's crumbs leave it free.
    pub(crate) fn keep_status_trailing(ui: &egui::Ui, flow_end: f32, right_block: f32) {
        let ctx = ui.ctx();
        let end = ctx.data(|data| data.get_temp::<(u64, f32)>(egui::Id::unique(END_KEY)));
        if let Some((pass, end)) = end
            && pass == ctx.cumulative_pass_nr()
        {
            let trailing = (flow_end - end).max(0.0) + right_block + ui.spacing().item_spacing.x;
            ctx.data_mut(|data| data.insert_temp(egui::Id::unique(TRAILING_KEY), trailing));
        }
    }
}

/// Room for the crumbs: the rest of the row less what the status bar took
/// after them on the last frame.
fn crumbs_budget(ui: &egui::Ui) -> f32 {
    let trailing = ui
        .ctx()
        .data(|data| data.get_temp::<f32>(egui::Id::unique(TRAILING_KEY)));
    ui.available_width() - trailing.unwrap_or(0.0)
}

/// The texts of one breadcrumb row.
struct CrumbTexts<'a> {
    project: &'a str,
    /// The folders, then the file.
    segments: &'a [&'a str],
    symbol: Option<&'a str>,
    busy: Option<&'a str>,
}

/// Painted widths of the breadcrumb pieces, each with its item spacing.
#[derive(Debug)]
struct CrumbWidths {
    /// The project name and the busy note: they always show.
    fixed: f32,
    /// A separator and a crumb for each folder, then for the file.
    segments: Vec<f32>,
    /// A separator and the `…` crumb that stands for the folded folders.
    fold: f32,
    /// A separator and the symbol crumb; zero without a symbol.
    symbol: f32,
}

/// Which crumbs show: how many leading folders fold into `…`, and whether
/// the caret's symbol shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CrumbFit {
    folded: usize,
    symbol: bool,
}

impl CrumbWidths {
    /// Widths as [`App::status_editor_crumbs`] paints them in `ui`.
    fn measure(ui: &egui::Ui, texts: &CrumbTexts<'_>) -> Self {
        let font = egui::TextStyle::Body.resolve(ui.style());
        let gap = ui.spacing().item_spacing.x;
        let text = |text: &str| {
            let galley =
                ui.painter()
                    .layout_no_wrap(text.to_owned(), font.clone(), Color32::PLACEHOLDER);
            galley.size().x + gap
        };
        let crumb = |label: &str| text("›") + text(label) + CRUMB_PAD.x;
        Self {
            fixed: text(texts.project) + texts.busy.map_or(0.0, text),
            segments: texts.segments.iter().map(|label| crumb(label)).collect(),
            fold: crumb("…"),
            symbol: texts.symbol.map_or(0.0, crumb),
        }
    }

    fn width(&self, fit: CrumbFit) -> f32 {
        let shown = self.segments.iter().skip(fit.folded).sum::<f32>();
        let fold = if fit.folded > 0 { self.fold } else { 0.0 };
        let symbol = if fit.symbol { self.symbol } else { 0.0 };
        self.fixed + shown + fold + symbol
    }

    /// The most crumbs that fit `budget`: leading folders fold first, then
    /// the symbol hides. The project and the file always show, so a row
    /// too narrow even for them still overflows.
    fn fit(&self, budget: f32) -> CrumbFit {
        let folders = self.segments.len().saturating_sub(1);
        [true, false]
            .into_iter()
            .flat_map(|symbol| (0..=folders).map(move |folded| CrumbFit { folded, symbol }))
            .find(|fit| self.width(*fit) <= budget)
            .unwrap_or(CrumbFit {
                folded: folders,
                symbol: false,
            })
    }
}

/// One breadcrumb in muted text. A crumb that acts highlights on hover,
/// like the `JetBrains` navigation bar.
fn crumb(ui: &mut egui::Ui, text: &str, acts: bool) -> egui::Response {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let muted = ui.visuals().weak_text_color();
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, muted);
    let sense = if acts {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let size = galley.size();
    let padded = egui::vec2(size.x + CRUMB_PAD.x, size.y + CRUMB_PAD.y);
    let (rect, response) = ui.allocate_exact_size(padded, sense);
    let hovered = acts && (response.hovered() || response.is_pointer_button_down_on());
    if hovered {
        let fill = ui.visuals().widgets.hovered.weak_bg_fill;
        ui.painter()
            .rect_filled(rect, appearance::WIDGET_RADIUS, fill);
    }
    let color = if hovered {
        ui.visuals().text_color()
    } else {
        muted
    };
    let center = rect.center();
    let at = egui::pos2(center.x - size.x / 2.0, center.y - size.y / 2.0);
    ui.painter()
        .galley_with_override_text_color(at, galley, color);
    let role = if acts {
        egui::Role::Button
    } else {
        egui::Role::Label
    };
    response.widget_info(|| egui::WidgetInfo::labeled(role, true, text));
    response
}

#[cfg(test)]
mod tests {
    use super::{CrumbFit, CrumbWidths};
    #[cfg(feature = "test-support")]
    use crate::*;

    fn fit(folded: usize, symbol: bool) -> CrumbFit {
        CrumbFit { folded, symbol }
    }

    /// The whole status bar in IDE mode, with a native editor on a deep
    /// path: after a frame to measure the row, the crumbs fold and the bell
    /// stays clear of the right-aligned controls.
    #[cfg(feature = "test-support")]
    #[test]
    fn a_deep_path_folds_so_the_status_controls_keep_their_place() {
        let (mut app, ctx, _dir) = crate::app::tests::support::fixture();
        app.preferences.ide_mode = true;
        let path = PathBuf::from("/a/crates/app/src/native_editor/view/toolbar.rs");
        let tab = Tab::NativeEditor { path: path.clone() };
        let dock = egui_dock::DockState::new(vec![tab]);
        app.layouts.insert("a".into(), Workspace::from_layout(dock));
        let text = "fn markdown_toolbar_fade() {}\n";
        let doc = crate::native_editor::NativeDoc::loaded(path.clone(), text);
        app.native_docs.insert(path, doc);
        for _ in 0..3 {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 40.0));
            let input = egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| app.status_bar(ui));
            output.textures_delta.clear();
        }
        let rect = |name: &str| crate::diagnostics::current_rect(&ctx, name);
        let crumbs = rect("status-breadcrumb-project");
        let bell = rect("status-attention-bell").expect("bell");
        let height = rect("status-sidebar-height").expect("sidebar height toggle");
        assert!(
            bell.right() < height.left(),
            "{crumbs:?} {bell:?} {height:?}"
        );
        // Measuring the right-aligned block does not move it off the end.
        let toggle = rect("status-terminal-toggle").expect("terminal strip toggle");
        assert!((toggle.right() - 1000.0).abs() < 0.5, "{toggle:?}");
        assert!(rect("status-crumb-fold").is_some(), "{crumbs:?} {bell:?}");
        assert!(rect("status-crumb:toolbar.rs").is_some());
    }

    #[test]
    fn crumbs_fold_leading_folders_before_the_symbol_hides() {
        // Three folders and the file; everything takes 20 + 180 + 70 = 270.
        let widths = CrumbWidths {
            fixed: 20.0,
            segments: vec![50.0, 40.0, 30.0, 60.0],
            fold: 15.0,
            symbol: 70.0,
        };
        assert_eq!(widths.fit(270.0), fit(0, true));
        // `…` stands for the first folder: 20 + 15 + 130 + 70 = 235.
        assert_eq!(widths.fit(240.0), fit(1, true));
        // Every folder folded: 20 + 15 + 60 + 70 = 165.
        assert_eq!(widths.fit(165.0), fit(3, true));
        // Then the symbol hides, and folders come back as room allows.
        assert_eq!(widths.fit(150.0), fit(2, false));
        // The project and the file show even when they do not fit.
        assert_eq!(widths.fit(10.0), fit(3, false));
    }

    #[test]
    fn a_single_segment_path_only_hides_the_symbol() {
        let widths = CrumbWidths {
            fixed: 20.0,
            segments: vec![200.0],
            fold: 15.0,
            symbol: 70.0,
        };
        assert_eq!(widths.fit(290.0), fit(0, true));
        assert_eq!(widths.fit(250.0), fit(0, false));
    }
}
