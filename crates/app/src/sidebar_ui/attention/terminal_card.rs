//! A terminal notice (OSC 9/777, OSC 7501 program status, quota and
//! long-command notices) painted as an attention card. Display only: the
//! glyph and brand come from the notice title, never from agent lifecycle,
//! and the card offers only Go and Dismiss.
use super::card::{CardFace, CardGlyph, CardVerbs, paint_card};
use super::*;
use terminator_core::TerminalNotice;

/// How one terminal notice card is drawn.
pub(crate) struct TerminalCard<'a> {
    pub theme: &'a AppearanceConfig,
    pub notice: &'a TerminalNotice,
    pub session: Option<&'a Session>,
    pub project: Option<&'a str>,
    pub selected: bool,
    /// The daemon takes Dismiss (`terminal-notices-v1`).
    pub dismiss: bool,
}

/// What a terminal notice title says, for the titles that
/// `program_status::notice` writes ("codex is done", "codex is waiting
/// (permission)"). Any other title has no state and no program.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct TerminalNoticeKind<'a> {
    pub state: Option<AgentState>,
    pub program: Option<&'a str>,
}

pub(crate) fn terminal_notice_kind(title: &str) -> TerminalNoticeKind<'_> {
    let none = TerminalNoticeKind {
        state: None,
        program: None,
    };
    let (program, state) = if let Some(program) = title.strip_suffix(" is done") {
        (program, AgentState::Completed)
    } else if let Some((program, rest)) = title.split_once(" is waiting")
        && (rest.is_empty() || (rest.starts_with(" (") && rest.ends_with(')')))
    {
        (program, AgentState::WaitingInput)
    } else {
        return none;
    };
    if program.trim().is_empty() {
        return none;
    }
    TerminalNoticeKind {
        state: Some(state),
        program: Some(program),
    }
}

/// The catalog agent named by a program name: its kind, exe or display name.
pub(crate) fn program_agent(program: &str) -> Option<&'static str> {
    let program = program.trim();
    terminator_core::agents::CATALOG
        .iter()
        .find(|info| {
            info.kind.eq_ignore_ascii_case(program)
                || info.name.eq_ignore_ascii_case(program)
                || info
                    .exes
                    .iter()
                    .any(|exe| exe.eq_ignore_ascii_case(program))
        })
        .map(|info| info.kind)
}

/// "title — body" on one line, whitespace collapsed.
pub(crate) fn terminal_notice_text(notice: &TerminalNotice) -> String {
    let join = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    match (join(&notice.title), join(&notice.body)) {
        (title, body) if title.is_empty() && body.is_empty() => "Terminal".to_owned(),
        (title, body) if body.is_empty() => title,
        (title, body) if title.is_empty() => body,
        (title, body) => format!("{title} — {body}"),
    }
}

fn terminal_glyph(state: Option<AgentState>, theme: &AppearanceConfig) -> CardGlyph {
    const DETAIL: &str = "Reported by the program in this terminal. Display only.";
    match state {
        Some(state) => CardGlyph {
            label: if state == AgentState::Completed {
                "Done"
            } else {
                "Waiting"
            },
            detail: DETAIL,
            ..CardGlyph::state(state, theme)
        },
        None => CardGlyph {
            icon: "Bell",
            tint: appearance::color(&theme.secondary),
            label: "Terminal notice",
            detail: DETAIL,
        },
    }
}

/// Paints one terminal notice card; returns its action and its rect.
pub(crate) fn terminal_card(
    ui: &mut egui::Ui,
    input: &TerminalCard<'_>,
) -> (AttentionAction, egui::Rect) {
    let notice = input.notice;
    let kind = terminal_notice_kind(&notice.title);
    let agent = kind.program.and_then(program_agent);
    let (brand_icon, brand_label, brand_tint) = match agent {
        // Brand glyphs carry their own colors; white leaves them as-is.
        Some(agent) => (
            terminator_core::agents::icon_key(agent),
            terminator_core::agents::display_name(agent),
            Color32::WHITE,
        ),
        None => ("Terminal", "Terminal", appearance::icon_color()),
    };
    let face = CardFace {
        theme: input.theme,
        session_id: &notice.session_id,
        session: input.session,
        project: input.project,
        selected: input.selected,
        highlight: false,
        glyph: terminal_glyph(kind.state, input.theme),
        created: (notice.created > 0).then_some(notice.created),
        text: terminal_notice_text(notice),
        resolved: false,
        brand_icon: Some(brand_icon),
        brand_label: Some(brand_label),
        brand_tint,
        pinned: false,
        usage: &[],
        extra: &[],
        target: "terminal",
    };
    let mut buttons = vec![("ArrowRight", "Go to terminal", "go", AttentionAction::Go)];
    if input.dismiss {
        buttons.push(("X", "Dismiss", "dismiss", AttentionAction::Dismiss));
    }
    let verbs = CardVerbs {
        buttons,
        keys: |_: &egui::Ui| AttentionAction::None,
        menu: |_: &egui::Response| AttentionAction::None,
    };
    ui.push_id(("terminal-notice", &notice.id), |ui| {
        paint_card(ui, &face, verbs)
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(title: &str, body: &str) -> TerminalNotice {
        TerminalNotice {
            id: "tn".into(),
            session_id: "s".into(),
            title: title.into(),
            body: body.into(),
            created: 1,
            dismissed: false,
        }
    }

    /// Paints one card per frame; returns each frame's action.
    #[cfg(feature = "test-support")]
    fn paint(
        ctx: &egui::Context,
        dismiss: bool,
        frames: Vec<Vec<egui::Event>>,
    ) -> Vec<AttentionAction> {
        let theme = AppearanceConfig::default();
        let notice = notice("codex is done", "ok");
        let mut actions = Vec::new();
        for events in frames {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(300.0, 200.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let input = TerminalCard {
                        theme: &theme,
                        notice: &notice,
                        session: None,
                        project: None,
                        selected: false,
                        dismiss,
                    };
                    actions.push(terminal_card(ui, &input).0);
                },
            );
            output.textures_delta.clear();
        }
        actions
    }

    #[cfg(feature = "test-support")]
    fn target(ctx: &egui::Context, name: &str) -> Option<egui::Rect> {
        ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("fixture-target", name))))
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn terminal_card_offers_go_and_dismiss_only_with_capability() {
        let ctx = egui::Context::default();
        paint(&ctx, false, vec![vec![]]);
        assert!(target(&ctx, "terminal-row:s").is_some());
        assert!(target(&ctx, "terminal-go:s").is_some());
        assert!(target(&ctx, "terminal-dismiss:s").is_none());
        for verb in ["snooze", "pin", "read", "review"] {
            assert!(
                target(&ctx, &format!("terminal-{verb}:s")).is_none(),
                "{verb}"
            );
        }
        let ctx = egui::Context::default();
        paint(&ctx, true, vec![vec![]]);
        assert!(target(&ctx, "terminal-dismiss:s").is_some());
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn click_on_terminal_card_goes_to_the_terminal() {
        let ctx = egui::Context::default();
        paint(&ctx, true, vec![vec![]]);
        let row = target(&ctx, "terminal-row:s").unwrap();
        let at = egui::pos2(row.left() + 4.0, row.bottom() + 8.0);
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let actions = paint(
            &ctx,
            true,
            vec![
                vec![egui::Event::PointerMoved(at)],
                vec![button(true)],
                vec![button(false)],
            ],
        );
        assert!(actions.contains(&AttentionAction::Go));
        assert!(!actions.contains(&AttentionAction::Dismiss));
    }

    #[test]
    fn program_status_titles_show_done_and_waiting_glyphs() {
        assert_eq!(
            terminal_notice_kind("codex is done"),
            TerminalNoticeKind {
                state: Some(AgentState::Completed),
                program: Some("codex"),
            }
        );
        assert_eq!(
            terminal_notice_kind("codex is waiting (permission)").state,
            Some(AgentState::WaitingInput)
        );
        assert_eq!(
            terminal_notice_kind("Program is waiting").program,
            Some("Program")
        );
    }

    #[test]
    fn other_terminal_titles_carry_no_state() {
        for title in ["Terminal", "", " is done", "build is waiting for disk"] {
            assert_eq!(
                terminal_notice_kind(title),
                TerminalNoticeKind {
                    state: None,
                    program: None,
                },
                "{title}"
            );
        }
    }

    #[test]
    fn known_program_names_map_to_agent_brands() {
        assert_eq!(program_agent("codex"), Some("codex"));
        assert_eq!(program_agent("Codex"), Some("codex"));
        assert_eq!(program_agent("Program"), None);
        assert_eq!(program_agent("make"), None);
    }

    #[test]
    fn notice_text_joins_title_and_body_on_one_line() {
        assert_eq!(
            terminal_notice_text(&notice("codex is done", "Fixed\n the bug")),
            "codex is done — Fixed the bug"
        );
        assert_eq!(terminal_notice_text(&notice("", "bell")), "bell");
        assert_eq!(terminal_notice_text(&notice("Done", " ")), "Done");
        assert_eq!(terminal_notice_text(&notice("", "")), "Terminal");
    }
}
