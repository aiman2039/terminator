//! Git log commit rows: the lane graph, ref pills, the subject, and a weak
//! hash, date, and author line. The lanes come with the loaded log; only
//! the visible rows are laid out and painted.
use crate::appearance;
use crate::egui::{self, Color32, Pos2, Rect, Stroke, pos2, vec2};
use crate::git_log::{CommitEntry, CommitLog, GraphRow, RefLabel, RefLabelKind};
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use std::sync::Arc;
use terminator_core::appearance::AppearanceConfig;

/// Distance between lane centers.
const LANE_PITCH: f32 = 12.0;
/// The narrowest lane pitch; a graph wider than that is clipped.
const MIN_LANE_PITCH: f32 = 5.0;
/// The largest share of the row the graph may take.
const MAX_GRAPH_SHARE: f32 = 0.4;
const LINE_WIDTH: f32 = 1.5;
const DOT_RADIUS: f32 = 3.5;
/// Space between the graph and the text, and after the text.
const GAP: f32 = 6.0;
const ROW_PADDING: f32 = 4.0;
const LINE_GAP: f32 = 1.0;
const PILL_FONT: f32 = appearance::FONT_CAPTION;
const PILL_PADDING: f32 = 5.0;
const PILL_GAP: f32 = 4.0;
/// The longest pill text before an ellipsis cuts it.
const MAX_PILL_TEXT: f32 = 140.0;
/// Less room than this shows a `+N` pill instead.
const MIN_PILL_TEXT: f32 = 24.0;
/// Room kept for the `+N` pill while more labels follow.
const MORE_PILL_WIDTH: f32 = 30.0;
/// The largest share of the subject line that pills may take.
const MAX_PILLS_SHARE: f32 = 0.6;
const PILL_FILL: f32 = 0.22;
const PILL_EDGE: f32 = 0.75;

/// The commit list's read-only inputs for one frame.
pub(super) struct CommitList<'a> {
    pub(super) log: &'a CommitLog,
    pub(super) theme: &'a AppearanceConfig,
    pub(super) selected: Option<&'a str>,
    /// Scroll so this row is in the middle of the list, for one frame.
    pub(super) reveal: Option<usize>,
}

/// One row to paint.
struct RowSpec<'a> {
    commit: &'a CommitEntry,
    lanes: LaneGeometry,
    height: f32,
    selected: bool,
}

impl CommitList<'_> {
    /// The scrolling list; returns the clicked commit's hash.
    pub(super) fn show(&self, ui: &mut egui::Ui) -> Option<String> {
        let height = row_height(ui);
        let total = self.log.commits.len();
        ui.scope(|ui| {
            // Rows touch, so lane lines run unbroken from row to row.
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut area = appearance::sidebar_scroll("commits");
            if let Some(row) = self.reveal {
                let top = row as f32 * height - (ui.available_height() - height) * 0.5;
                area = area.vertical_scroll_offset(top.max(0.0));
            }
            area.show_rows(ui, height, total, |ui, range| self.rows(ui, range, height))
                .inner
        })
        .inner
    }

    fn rows(
        &self,
        ui: &mut egui::Ui,
        range: std::ops::Range<usize>,
        height: f32,
    ) -> Option<String> {
        let lanes = LaneGeometry::new(self.log.lanes, ui.available_width());
        let mut clicked = None;
        for commit in self.log.commits.get(range).unwrap_or_default() {
            let selected = self.selected == Some(commit.hash.as_str());
            let spec = RowSpec {
                commit,
                lanes,
                height,
                selected,
            };
            if self.row(ui, spec).clicked() {
                clicked = Some(commit.hash.clone());
            }
        }
        clicked
    }

    fn row(&self, ui: &mut egui::Ui, spec: RowSpec<'_>) -> egui::Response {
        let RowSpec {
            commit,
            lanes,
            height,
            selected,
        } = spec;
        let size = vec2(ui.available_width(), height);
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::Button, true, selected, &commit.subject)
        });
        if !ui.is_rect_visible(rect) {
            return response;
        }
        if selected || response.hovered() {
            let fill = if selected {
                ui.visuals().selection.bg_fill
            } else {
                ui.visuals().widgets.hovered.bg_fill
            };
            ui.painter()
                .rect_filled(rect, appearance::RADIUS_SMALL, fill);
        }
        // Text starts right after this row's own lanes, like `git log --graph`.
        let width = lanes.row_width(&commit.graph);
        let graph = Rect::from_min_size(rect.min, vec2(width, height));
        paint_graph(&ui.painter_at(graph), graph, &commit.graph, lanes);
        let text = Rect::from_min_max(
            pos2(graph.right() + GAP, rect.top() + ROW_PADDING),
            pos2(rect.right() - GAP, rect.bottom() - ROW_PADDING),
        );
        self.paint_text(ui, commit, text);
        if commit.refs.is_empty() {
            return response;
        }
        // Every label, also the ones cut short or folded into `+N`.
        response.on_hover_ui(|ui| {
            let names: Vec<&str> = commit.refs.iter().map(|r| r.name.as_str()).collect();
            ui.label(names.join(", "));
        })
    }

    /// Line one: ref pills, then the subject. Line two: hash, date, author.
    fn paint_text(&self, ui: &egui::Ui, commit: &CommitEntry, rect: Rect) {
        let painter = ui.painter_at(rect);
        let line_height = ui.text_style_height(&egui::TextStyle::Body);
        let first = Rect::from_min_size(rect.min, vec2(rect.width(), line_height));
        let text = ui.visuals().text_color();
        let mut pills = Pills {
            painter: &painter,
            line: first,
            x: first.left(),
            text,
        };
        pills.labels(&commit.refs, self.theme);
        let body = TextFormat::simple(egui::TextStyle::Body.resolve(ui.style()), text);
        let subject = truncated(&painter, &commit.subject, body, first.right() - pills.x);
        painter.galley(pos2(pills.x, first.top()), subject, text);
        let weak = ui.visuals().weak_text_color();
        let small = TextFormat::simple(egui::TextStyle::Small.resolve(ui.style()), weak);
        let meta = format!("{}  {}  {}", commit.short_hash, commit.date, commit.author);
        let meta = truncated(&painter, &meta, small, rect.width());
        painter.galley(pos2(rect.left(), first.bottom() + LINE_GAP), meta, weak);
    }
}

/// Two text lines and padding.
fn row_height(ui: &egui::Ui) -> f32 {
    let lines = ui.text_style_height(&egui::TextStyle::Body)
        + ui.text_style_height(&egui::TextStyle::Small);
    (lines + LINE_GAP + 2.0 * ROW_PADDING).ceil()
}

/// `text` on one line, cut with an ellipsis past `width`.
fn truncated(
    painter: &egui::Painter,
    text: &str,
    format: TextFormat,
    width: f32,
) -> Arc<egui::Galley> {
    let mut job = LayoutJob::single_section(text.to_owned(), format);
    job.wrap = TextWrapping::truncate_at_width(width.max(0.0));
    painter.layout_job(job)
}

/// Lane spacing for one frame, the same for every row so lanes line up:
/// the full pitch while the widest row fits its share of the row, then
/// narrower; past the narrowest pitch the graph clips at that share.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct LaneGeometry {
    pub(super) pitch: f32,
    pub(super) max_width: f32,
}

impl LaneGeometry {
    pub(super) fn new(lanes: usize, row_width: f32) -> Self {
        let count = lanes.max(1) as f32;
        let max_width = (row_width * MAX_GRAPH_SHARE).max(LANE_PITCH);
        let pitch = (max_width / count).clamp(MIN_LANE_PITCH, LANE_PITCH);
        Self { pitch, max_width }
    }

    /// The graph width of one row: its own lanes, up to the share.
    pub(super) fn row_width(self, row: &GraphRow) -> f32 {
        (row.width() as f32 * self.pitch).min(self.max_width)
    }

    fn x(self, left: f32, col: u16) -> f32 {
        left + self.pitch * (f32::from(col) + 0.5)
    }
}

/// One row's lane lines, then the commit dot on top of them.
fn paint_graph(painter: &egui::Painter, rect: Rect, row: &GraphRow, lanes: LaneGeometry) {
    let x = |col: u16| lanes.x(rect.left(), col);
    let (top, bottom) = (rect.top(), rect.bottom());
    let dot = pos2(x(row.col), rect.center().y);
    for line in &row.through {
        let lane = x(line.col);
        painter.line_segment(
            [pos2(lane, top), pos2(lane, bottom)],
            lane_stroke(line.color),
        );
    }
    for line in &row.up {
        edge(
            painter,
            pos2(x(line.col), top),
            dot,
            lane_stroke(line.color),
        );
    }
    for line in &row.down {
        edge(
            painter,
            dot,
            pos2(x(line.col), bottom),
            lane_stroke(line.color),
        );
    }
    let radius = (lanes.pitch * 0.3).min(DOT_RADIUS);
    painter.circle_filled(dot, radius, appearance::lane_color(row.color));
}

/// A straight line within one lane, an S-curve from one lane to another.
fn edge(painter: &egui::Painter, from: Pos2, to: Pos2, stroke: Stroke) {
    if (from.x - to.x).abs() < 0.5 {
        painter.line_segment([from, to], stroke);
        return;
    }
    let bend = (to.y - from.y) * 0.5;
    painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
        [
            from,
            pos2(from.x, from.y + bend),
            pos2(to.x, to.y - bend),
            to,
        ],
        false,
        Color32::TRANSPARENT,
        stroke,
    ));
}

fn lane_stroke(color: u8) -> Stroke {
    Stroke::new(LINE_WIDTH, appearance::lane_color(color))
}

/// Theme colors per ref kind: HEAD amber, local branches green, remote
/// branches the accent blue, tags the secondary gray.
fn label_color(theme: &AppearanceConfig, kind: RefLabelKind) -> Color32 {
    appearance::color(match kind {
        RefLabelKind::Head => &theme.status_waiting,
        RefLabelKind::LocalBranch => &theme.git_added,
        RefLabelKind::RemoteBranch => &theme.accent,
        RefLabelKind::Tag => &theme.secondary,
    })
}

/// Paints ref pills left to right along one text line.
struct Pills<'a> {
    painter: &'a egui::Painter,
    line: Rect,
    /// Where the next pill, or the subject, starts.
    x: f32,
    text: Color32,
}

/// One pill: `name` cut to `room`, tinted `color`.
struct Pill<'a> {
    name: &'a str,
    room: f32,
    color: Color32,
    /// Tags get square corners, branches round ones.
    tag: bool,
}

impl Pills<'_> {
    /// Pills for `labels` within their share of the line; labels that do
    /// not fit fold into one `+N` pill.
    fn labels(&mut self, labels: &[RefLabel], theme: &AppearanceConfig) {
        let limit = self.line.left() + self.line.width() * MAX_PILLS_SHARE;
        for (index, label) in labels.iter().enumerate() {
            let last = index.saturating_add(1) == labels.len();
            let reserve = if last { 0.0 } else { MORE_PILL_WIDTH };
            let room = (limit - self.x - reserve - 2.0 * PILL_PADDING).min(MAX_PILL_TEXT);
            if room < MIN_PILL_TEXT {
                let more = format!("+{}", labels.len().saturating_sub(index));
                self.add(Pill {
                    name: &more,
                    room: MAX_PILL_TEXT,
                    color: appearance::color(&theme.secondary),
                    tag: false,
                });
                return;
            }
            self.add(Pill {
                name: &label.name,
                room,
                color: label_color(theme, label.kind),
                tag: label.kind == RefLabelKind::Tag,
            });
        }
    }

    fn add(&mut self, pill: Pill<'_>) {
        let font = TextFormat::simple(egui::FontId::proportional(PILL_FONT), self.text);
        let galley = truncated(self.painter, pill.name, font, pill.room);
        let height = (galley.size().y + 2.0).min(self.line.height());
        let rect = Rect::from_min_size(
            pos2(self.x, self.line.center().y - height * 0.5),
            vec2(galley.size().x + 2.0 * PILL_PADDING, height),
        );
        let radius = if pill.tag {
            f32::from(appearance::RADIUS_XSMALL)
        } else {
            height * 0.5
        };
        self.painter.rect(
            rect,
            radius,
            pill.color.gamma_multiply(PILL_FILL),
            Stroke::new(1.0, pill.color.gamma_multiply(PILL_EDGE)),
            egui::StrokeKind::Inside,
        );
        let top = rect.center().y - galley.size().y * 0.5;
        self.painter
            .galley(pos2(rect.left() + PILL_PADDING, top), galley, self.text);
        self.x = rect.right() + PILL_GAP;
    }
}
