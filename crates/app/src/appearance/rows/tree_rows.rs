//! Project-tree rows: project, tab group and worktree rows, and session
//! rows. Each can carry [`RowExtras`]: state counters, a colored second
//! line, and state-colored trailing words.
use super::counters::{RowExtras, RowSelection};
use super::list::{RowSpec, row_ext};
use super::scale::{FONT_LARGE, semibold};
use eframe::egui::{self, Color32, FontFamily, FontId, TextStyle};

pub(crate) struct ProjectRowInput<'a> {
    pub ui: &'a mut egui::Ui,
    pub label: &'a str,
    pub icon: &'a str,
    /// Label and icon color; `None` keeps the theme colors.
    pub color: Option<Color32>,
    pub selected: bool,
    pub height: f32,
    pub trailing: &'a str,
    pub tint: Color32,
}

/// A project, tab group or worktree row, with optional state counters,
/// a count pill or a second line.
///
/// A project-tree node never takes the selection fill: only the active
/// session row is filled. A selected (open) node shows semibold text.
pub fn project_row_with(input: ProjectRowInput<'_>, extras: RowExtras<'_>) -> egui::Response {
    let ProjectRowInput {
        ui,
        label,
        icon,
        color,
        selected,
        height,
        trailing,
        tint,
    } = input;
    ui.scope(|ui| {
        let family = if selected {
            semibold()
        } else {
            FontFamily::Proportional
        };
        ui.style_mut()
            .text_styles
            .insert(TextStyle::Body, FontId::new(FONT_LARGE, family));
        if color.is_some() {
            ui.visuals_mut().override_text_color = color;
        }
        row_ext(
            ui,
            RowSpec {
                label,
                icon,
                selected,
                height,
                trailing,
                tint,
                icon_tint: color,
                spin: false,
                subtitle: None,
                brand: None,
                reserve_brand: false,
                after_label: None,
                extras: RowExtras {
                    selection: RowSelection::Text,
                    ..extras
                },
            },
        )
    })
    .inner
}

/// Session row with an optional status-tinted (and spinning) icon and a
/// muted second line. The subtitle gets its own line; it never overlaps the label.
/// `brand` paints the stable agent glyph beside the status icon.
pub struct SessionRowSpec<'a> {
    pub label: &'a str,
    pub icon: &'a str,
    pub selected: bool,
    /// The active session: its selected row also gets the accent bar.
    pub current: bool,
    pub trailing: &'a str,
    pub tint: Color32,
    pub icon_tint: Option<Color32>,
    pub spin: bool,
    pub subtitle: Option<&'a str>,
    pub brand: Option<&'a str>,
    /// The live-state dot at the right edge (the pane header's dot).
    pub status_dot: Option<Color32>,
}

pub const SESSION_ROW_HEIGHT: f32 = 24.0;
pub const SESSION_ROW_DETAIL_HEIGHT: f32 = 38.0;

/// A session row. Its trailing state words ("waiting 4m", "working") keep
/// the state tint.
pub fn session_row_spec(ui: &mut egui::Ui, spec: SessionRowSpec<'_>) -> egui::Response {
    session_row_with(
        ui,
        spec,
        RowExtras {
            tint_words: true,
            ..RowExtras::default()
        },
    )
}

/// A [`session_row_spec`] with state counters (a project row that shows
/// its agent face).
pub fn session_row_with(
    ui: &mut egui::Ui,
    spec: SessionRowSpec<'_>,
    extras: RowExtras<'_>,
) -> egui::Response {
    ui.scope(|ui| {
        if !spec.selected {
            ui.visuals_mut().override_text_color = Some(ui.visuals().weak_text_color());
        }
        row_ext(
            ui,
            RowSpec {
                label: spec.label,
                icon: spec.icon,
                selected: spec.selected,
                height: if spec.subtitle.is_some() {
                    SESSION_ROW_DETAIL_HEIGHT
                } else {
                    SESSION_ROW_HEIGHT
                },
                trailing: spec.trailing,
                tint: spec.tint,
                icon_tint: spec.icon_tint,
                spin: spec.spin,
                subtitle: spec.subtitle,
                brand: spec.brand,
                reserve_brand: true,
                after_label: None,
                extras: RowExtras {
                    selection: if spec.current {
                        RowSelection::Bar
                    } else {
                        RowSelection::Fill
                    },
                    status_dot: spec.status_dot,
                    ..extras
                },
            },
        )
    })
    .inner
}
