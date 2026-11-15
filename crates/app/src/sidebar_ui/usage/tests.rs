//! Usage tool: reads only while shown and shows `n/a` for silent agents.
use super::*;
use crate::app::tests::support::painted_strings;
use terminator_core::agents::quota::QuotaWindow;

fn draw(app: &mut App, ctx: &egui::Context) -> Vec<String> {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(320.0, 600.0),
            )),
            ..Default::default()
        },
        |ui| app.usage_panel(ui),
    );
    output.textures_delta.clear();
    painted_strings(&output.shapes)
}

#[test]
fn usage_is_read_only_while_the_tool_shows_or_an_alert_is_on() {
    let (mut app, ctx, _dir) = crate::app::fixture();
    draw(&mut app, &ctx);
    app.sync_quota_request();
    assert!(*app.quota_jobs.borrow());
    app.sync_quota_request();
    assert!(!*app.quota_jobs.borrow());
    app.preferences.quota_alerts.ntfy = true;
    app.sync_quota_request();
    assert!(*app.quota_jobs.borrow());
}

#[test]
fn reported_windows_show_and_a_silent_agent_shows_n_a() {
    let (mut app, ctx, _dir) = crate::app::fixture();
    app.handle_quota(vec![Quota {
        agent: "codex".into(),
        plan: Some("pro".into()),
        windows: vec![QuotaWindow {
            label: "5h".into(),
            used_percent: 42.0,
            resets_at: None,
        }],
        observed_at: 0,
    }]);
    let texts = draw(&mut app, &ctx);
    assert!(texts.iter().any(|t| t == "5h 42%"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "pro"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "n/a"), "{texts:?}");
}

fn claude(used: f64, resets_at: Option<u64>) -> Quota {
    Quota {
        agent: "claude".into(),
        plan: None,
        windows: vec![QuotaWindow {
            label: "5h".into(),
            used_percent: used,
            resets_at,
        }],
        observed_at: 1,
    }
}

#[test]
fn alerts_fire_only_when_a_limit_is_reached_or_resets() {
    let titles = |before: &Quota, after: &Quota| {
        quota_alerts(
            std::slice::from_ref(before),
            std::slice::from_ref(after),
            100,
        )
        .into_iter()
        .map(|(_, title)| title)
        .collect::<Vec<_>>()
    };
    let (low, full) = (claude(80.0, Some(500)), claude(100.0, Some(500)));
    assert_eq!(titles(&low, &full), ["Claude Code: 5h limit reached"]);
    assert!(titles(&full, &full).is_empty(), "no repeat");
    assert_eq!(
        titles(&full, &claude(3.0, Some(900))),
        ["Claude Code: 5h window reset"]
    );
    // A full window whose reset time passed counts as reset.
    assert_eq!(
        titles(&full, &claude(100.0, Some(50))),
        ["Claude Code: 5h window reset"]
    );
    // The first read never alerts.
    assert!(quota_alerts(&[], &[full], 100).is_empty());
}

#[test]
fn an_opted_in_alert_goes_to_the_agents_live_terminal_once() {
    let (mut app, _ctx, _dir) = crate::app::fixture();
    let (jobs, received) = std::sync::mpsc::channel();
    app.jobs = jobs.into();
    app.state
        .capabilities
        .push(terminator_core::AGENT_QUOTA_CAPABILITY.into());
    app.state.sessions = vec![crate::app::tests::support::session_fixture(
        "s1",
        SessionKind::Shell,
    )];
    // The session belongs to the daemon that advertises the capability.
    app.state.generation = app.state.sessions[0].generation.clone();
    app.state.agents = serde_json::from_value(serde_json::json!([{
        "invocation_id": "i", "session_id": "s1", "kind": "claude",
        "provider_session_id": null, "state": "running", "sequence": null,
        "updated": 1, "resume": null
    }]))
    .unwrap();
    app.handle_quota(vec![claude(50.0, Some(500))]);
    app.handle_quota(vec![claude(100.0, Some(u64::MAX))]);
    assert!(received.try_recv().is_err(), "no channel ticked");
    app.preferences.quota_alerts.os = true;
    app.handle_quota(vec![claude(10.0, Some(u64::MAX))]);
    let Ok(Job::Control(request, _)) = received.try_recv() else {
        panic!("expected a QuotaNotify request");
    };
    let Request::QuotaNotify {
        session,
        title,
        os,
        inbox,
        ntfy,
    } = *request
    else {
        panic!("expected QuotaNotify");
    };
    assert_eq!(
        (session.as_str(), title.as_str()),
        ("s1", "Claude Code: 5h window reset")
    );
    assert!(os && !inbox && !ntfy);
}

#[test]
fn window_text_counts_down_to_the_reset() {
    let window = QuotaWindow {
        label: "weekly".into(),
        used_percent: 7.4,
        resets_at: Some(1_000 + 2 * 86_400 + 3 * 3600),
    };
    assert_eq!(window_text(&window, 1_000), "weekly 7% · resets in 2d 3h");
    assert_eq!(window_text(&window, 10_000_000), "weekly 7% · reset");
}
