//! Agent sessions window: resume only on an explicit pick, through the
//! CLI's own quoted resume command. The listing state is set directly, so
//! no test reads the user's agent files.
use super::*;
use crate::app::tests::support::{fixture, painted_strings};
use crate::{After, Job, PostCreate};
use eframe::egui;
use std::sync::mpsc;
use terminator_core::Request;

const CODEX_ID: &str = "01a1df77-e352-7740-8432-39622de34a19";

fn conversation(agent: &'static str, id: &str, title: &str) -> Conversation {
    Conversation {
        agent,
        id: id.into(),
        title: title.into(),
        updated: 0,
    }
}

/// The window open for project `a` with `rows` listed.
fn shown(rows: Vec<Conversation>) -> AgentSessions {
    AgentSessions {
        open: true,
        project: Some(("a".into(), "/a".into())),
        listing: Some(Listing {
            conversations: rows,
            stopped: false,
        }),
        ..AgentSessions::default()
    }
}

#[test]
fn resume_command_is_the_clis_own_quoted_command_for_a_plain_id() {
    let claude = conversation("claude", "1f0e9c2a-77b1", "t");
    assert_eq!(
        resume_command(&claude).as_deref(),
        Some("'claude' '--resume' '1f0e9c2a-77b1'")
    );
    let codex = conversation("codex", CODEX_ID, "t");
    assert_eq!(
        resume_command(&codex),
        Some(format!("'codex' 'resume' '{CODEX_ID}'"))
    );
    for bad in ["x; rm -rf ~", "$(id)", "-h", ""] {
        assert_eq!(
            resume_command(&conversation("claude", bad, "t")),
            None,
            "{bad}"
        );
    }
    assert_eq!(resume_command(&conversation("other", "abc", "t")), None);
}

#[test]
fn picking_a_row_opens_a_terminal_at_the_project_root_and_runs_resume_with_enter() {
    let (mut app, _, _dir) = fixture();
    let (jobs, received) = mpsc::channel();
    app.jobs = jobs.into();
    app.selected = Some("a".into());
    let root = app.selected_project().unwrap().path.clone();
    app.agent_sessions = shown(vec![conversation("codex", CODEX_ID, "t")]);
    app.resume_agent_session(&conversation("codex", CODEX_ID, "t"));
    let Job::Control(request, After::Workspace(id, _)) = received.try_recv().unwrap() else {
        panic!("Expected workspace create")
    };
    let Request::Create { project, cwd, .. } = *request else {
        panic!("Expected create")
    };
    assert_eq!((project.as_str(), cwd), ("a", Some(root)));
    assert_eq!(
        app.post_create.by_tab.get(&id),
        Some(&PostCreate {
            input: format!("'codex' 'resume' '{CODEX_ID}'"),
            enter: true,
        })
    );
    assert!(!app.agent_sessions.open, "the window closes after a pick");
}

#[test]
fn a_bad_id_or_another_selected_project_runs_nothing() {
    let (mut app, _, _dir) = fixture();
    let (jobs, received) = mpsc::channel();
    app.jobs = jobs.into();
    app.selected = Some("a".into());
    app.agent_sessions = shown(Vec::new());
    app.resume_agent_session(&conversation("claude", "a;b", "t"));
    assert!(received.try_recv().is_err());
    assert!(app.info.is_some());
    app.selected = Some("b".into());
    app.resume_agent_session(&conversation("claude", "abc", "t"));
    assert!(received.try_recv().is_err());
    assert!(!app.agent_sessions.open);
}

#[test]
fn the_filter_matches_title_or_agent_name_ignoring_case() {
    let mut sessions = shown(vec![
        conversation("claude", "a1", "Fix reconnect"),
        conversation("codex", "b2", "Explain the daemon"),
    ]);
    assert_eq!(sessions.matching(), [0, 1]);
    sessions.filter = "RECONNECT".into();
    assert_eq!(sessions.matching(), [0]);
    sessions.filter = "codex".into();
    assert_eq!(sessions.matching(), [1]);
}

fn draw(app: &mut App, ctx: &egui::Context) -> String {
    let mut text = String::new();
    for _ in 0..2 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 700.0),
                )),
                ..Default::default()
            },
            |ui| app.agent_sessions_dialog(ui.ctx()),
        );
        output.textures_delta.clear();
        text = painted_strings(&output.shapes).join("\n");
    }
    text
}

#[test]
fn the_window_shows_rows_or_an_empty_state_and_resumes_nothing_by_itself() {
    let (mut app, ctx, _dir) = fixture();
    let (jobs, received) = mpsc::channel();
    app.jobs = jobs.into();
    app.selected = Some("a".into());
    app.agent_sessions = shown(Vec::new());
    assert!(draw(&mut app, &ctx).contains("No Claude Code or Codex conversation"));
    app.agent_sessions = shown(vec![conversation("claude", "a1", "Fix reconnect")]);
    let painted = draw(&mut app, &ctx);
    assert!(painted.contains("Fix reconnect"), "{painted}");
    assert!(app.agent_sessions.open);
    assert!(received.try_recv().is_err(), "painting never resumes");
}

#[test]
fn row_dates_are_relative_for_a_week_then_a_day() {
    let now: u64 = 1_800_000_000;
    assert_eq!(view::when(now.saturating_sub(120), now), "2m ago");
    assert_eq!(view::when(now.saturating_sub(7200), now), "2h ago");
    assert_eq!(view::when(now.saturating_sub(172_800), now), "2d ago");
    assert_eq!(view::when(1_710_504_000, now), "2024-03-15");
}
