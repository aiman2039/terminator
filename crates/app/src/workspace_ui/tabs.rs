use super::super::*;
use egui_term::TerminalBackend;
pub(super) fn terminal_theme(
    theme: &AppearanceConfig,
) -> (String, String, egui_term::TerminalTheme) {
    (
        theme.terminal_background.clone(),
        theme.terminal_foreground.clone(),
        egui_term::TerminalTheme::new(Box::new(egui_term::ColorPalette {
            background: theme.terminal_background.clone(),
            foreground: theme.terminal_foreground.clone(),
            ..Default::default()
        })),
    )
}

pub(super) fn cached_terminal_theme(
    cache: &mut Option<(String, String, egui_term::TerminalTheme)>,
    theme: &AppearanceConfig,
) -> egui_term::TerminalTheme {
    let cached = cache.get_or_insert_with(|| terminal_theme(theme));
    if cached.0 != theme.terminal_background || cached.1 != theme.terminal_foreground {
        *cached = terminal_theme(theme);
    }
    cached.2.clone()
}

/// Last non-blank grid rows of a live terminal for the drag ghost, oldest
/// first. Bounded so the floating preview stays small.
/// Longest label before a workspace tab ellipsizes. Chrome around it is fixed.
pub(super) const TAB_TEXT_MAX: f32 = 160.0;

/// Width of a workspace tab for a measured label. Hugs the text instead of
/// a fixed 220px slot, and always reserves the close icon.
pub(super) fn workspace_tab_width(text_width: f32) -> f32 {
    let text = text_width.clamp(8.0, TAB_TEXT_MAX);
    // 30px to the label, then 4px, a 16px close icon, and 8px of padding.
    30.0 + text + 4.0 + 16.0 + 8.0
}

pub(super) fn tab_label_width(ui: &egui::Ui, label: &str) -> f32 {
    let mut text = egui::text::LayoutJob::simple(
        label.to_owned(),
        egui::FontId::proportional(13.0),
        egui::Color32::WHITE,
        TAB_TEXT_MAX,
    );
    text.wrap.max_rows = 1;
    text.wrap.break_anywhere = true;
    ui.painter().layout_job(text).size().x
}

/// Workspace tab hover tooltip: the label plus the session working
/// directory on a second line when the tab has a live session.
pub(super) fn tab_tooltip(label: &str, cwd: Option<&std::path::Path>) -> String {
    match cwd {
        Some(cwd) => format!("{label}\n{}", cwd.display()),
        None => label.to_owned(),
    }
}

/// Focused-terminal face for a workspace strip tab.
pub(super) struct TabFace {
    pub(super) label: String,
    pub(super) icon: &'static str,
    pub(super) sid: Option<String>,
    pub(super) brand: Option<&'static str>,
    pub(super) status: Option<(AgentState, &'static str, Color32)>,
}

/// Leading icons for one terminal tab, shared by the main-canvas caption,
/// the IDE strip's native tabs, and their width reservations. The brand is
/// the process-inspection identity and shows before any hook. Hook lifecycle
/// replaces the session-kind glyph once status exists. Plain shells keep the
/// kind glyph so every terminal tab carries an icon.
pub(super) struct TabLeading {
    pub(super) brand: Option<&'static str>,
    pub(super) status: Option<(&'static str, Color32, bool)>,
    pub(super) kind: Option<&'static str>,
}

impl TabLeading {
    pub(super) fn width(&self) -> f32 {
        // Status and the kind fallback never co-occur: one slot covers both.
        let icons = usize::from(self.brand.is_some())
            .saturating_add(usize::from(self.status.is_some() || self.kind.is_some()));
        f32::from(u16::try_from(icons).unwrap_or(u16::MAX)) * appearance::TERMINAL_LEADING_SLOT
    }
}

/// Terminal sessions in a top-level tab's split layout.
pub(super) fn group_terminal_ids(layout: &egui_dock::DockState<Tab>) -> Vec<String> {
    layout
        .iter_all_tabs()
        .filter_map(|(_, tab)| match tab {
            Tab::Terminal(sid) => Some(sid.clone()),
            _ => None,
        })
        .collect()
}

/// Aggregate attention badge text and tooltip breakdown for a workspace tab.
/// Input requests, permission requests, and failures stay distinct.
pub(super) fn tab_attention_text(
    counts: crate::agent_presence::AttentionCounts,
) -> Option<(String, String)> {
    if counts.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    if counts.input > 0 {
        parts.push(format!("{} input", counts.input));
    }
    if counts.permission > 0 {
        parts.push(format!("{} permission", counts.permission));
    }
    if counts.failed > 0 {
        parts.push(format!("{} failed", counts.failed));
    }
    Some((counts.total().to_string(), parts.join(" · ")))
}

pub(super) fn snapshot_rows(backend: &TerminalBackend) -> Vec<String> {
    let kept: Vec<String> = backend
        .search_rows()
        .into_iter()
        .map(|row| row.text.trim_end().replace('\t', "  "))
        .map(|line| line.chars().take(64).collect::<String>())
        .filter(|line| !line.trim().is_empty())
        .collect();
    let start = kept.len().saturating_sub(8);
    kept.into_iter().skip(start).collect()
}

#[cfg(test)]
pub(super) mod tab_tooltip_tests {
    use super::tab_tooltip;
    use std::path::Path;

    #[test]
    pub(super) fn tooltip_appends_the_working_directory() {
        assert_eq!(
            tab_tooltip("shell", Some(Path::new("/repo/proj"))),
            "shell\n/repo/proj"
        );
    }

    #[test]
    pub(super) fn tooltip_without_a_session_is_just_the_label() {
        assert_eq!(tab_tooltip("shell", None), "shell");
    }
}

#[cfg(test)]
pub(super) mod tab_width_tests {
    use super::workspace_tab_width;

    #[test]
    pub(super) fn tab_width_tracks_the_label_and_stays_under_the_old_slot() {
        let short = workspace_tab_width(8.0);
        let longer = workspace_tab_width(120.0);
        let capped = workspace_tab_width(400.0);
        assert!(short < longer);
        assert!(longer < 220.0);
        assert_eq!(capped, workspace_tab_width(160.0));
        assert!(capped < 220.0);
    }
}
