use super::CaptureInput;
use super::{Harness, Options, Result, Value, capture, ensure, id, json, prefs, save_prefs};
use terminator_core::{Paths, ui_control};

fn gui(h: &Harness) -> Result<Value> {
    ui_control::rpc(&Paths::at(h.root.clone()), ui_control::Request::Snapshot)
}

struct ToggleAgentsInput<'a> {
    h: &'a Harness,
    o: &'a Options,
    name: &'a str,
    open: bool,
    project: &'a Value,
}

fn toggle_agents(input: ToggleAgentsInput<'_>) -> Result<()> {
    let ToggleAgentsInput {
        h,
        o,
        name,
        open,
        project,
    } = input;
    let path = h.root.join("agent-toggle.json");
    terminator_core::atomic_write(&path, b"[]")?;
    let result = capture(
        CaptureInput {
            h,
            opts: o,
            name,
            actions: json!([]),
            after: 3500,
        },
        |_| {
            h.wait(
                |_| {
                    gui(h).is_ok_and(|s| {
                        (s.pointer("/controls/left-agent-bar")
                            .is_some_and(Value::is_array)
                            || s.pointer("/controls/project-header-menu")
                                .is_some_and(Value::is_array))
                            && s.get("agent_bar_badge")
                                .and_then(Value::as_str)
                                .is_some_and(|badge| badge.starts_with("1 waiting"))
                    })
                },
                5,
            )?;
            let snapshot = gui(h)?;
            let actions = if snapshot
                .pointer("/controls/left-agent-bar")
                .is_some_and(Value::is_array)
            {
                json!([{"at_ms":900,"target":"left-agent-bar"}])
            } else {
                ensure!(
                    snapshot
                        .pointer("/controls/project-header-menu")
                        .is_some_and(Value::is_array),
                    "Missing Agents button and header menu"
                );
                let badge = snapshot
                    .get("agent_bar_badge")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let label = if badge.is_empty() {
                    "Agents".to_owned()
                } else {
                    format!("Agents · {badge}")
                };
                json!([{"at_ms":900,"target":"project-header-menu"},{"at_ms":1400,"target":label}])
            };
            terminator_core::atomic_write(&path, &serde_json::to_vec(&actions)?)?;
            h.wait(
                |_| {
                    gui(h).is_ok_and(|s| {
                        s.get("left_agents").and_then(Value::as_bool) == Some(open)
                            && s.get("selected_project").and_then(Value::as_str)
                                == Some(id(project))
                            && s.get("agent_bar_badge")
                                .and_then(Value::as_str)
                                .is_some_and(|badge| badge.starts_with("1 waiting"))
                    })
                },
                8,
            )?;
            Ok(())
        },
    );
    let _ = std::fs::remove_file(path);
    result?;
    Ok(())
}

/// Two projects; the first holds an agent shell with a pending permission request.
struct Fixture {
    h: Harness,
    first: Value,
    second: Value,
    agent: Value,
    other: Value,
    event: Value,
}

pub fn run(o: &Options) -> Result<()> {
    let f = launch()?;
    f.waiting_projects(o)?;
    f.bell_open_on_git(o)?;
    toggle_agents(ToggleAgentsInput {
        h: &f.h,
        o,
        name: "agents-open",
        open: true,
        project: &f.second,
    })?;
    ensure!(
        prefs(&f.h)?
            .get("left_agents")
            .is_some_and(|value| value == &json!(true)),
        "Opening Agents did not persist"
    );
    f.agent_navigation(o)?;
    toggle_agents(ToggleAgentsInput {
        h: &f.h,
        o,
        name: "projects-restored",
        open: false,
        project: &f.first,
    })?;
    ensure!(
        prefs(&f.h)?
            .get("left_agents")
            .is_some_and(|value| value == &json!(false)),
        "Returning to Projects did not persist"
    );
    f.running(o)?;
    let Fixture {
        h, agent, other, ..
    } = f;
    h.assert_pids(&[agent, other])
}

fn launch() -> Result<Fixture> {
    let mut h = Harness::new()?;
    h.env.insert(
        "TERMINATOR_TEST_ACTIONS_PATH".into(),
        h.root
            .join("agent-toggle.json")
            .to_string_lossy()
            .into_owned(),
    );
    h.setup()?;
    // Keep the pending event through navigation so this fixture can test resolve updates.
    h.update_settings(|settings| {
        settings.insert("dismissal".into(), json!("Manual"));
        // The top attention bar records `attention`. Side mode hides that bar.
        settings.insert("notifications_side".into(), json!(false));
    })?;
    let first = h.project("agent-project")?;
    let second = h.project("other-project")?;
    let agent = h.shell(&first)?;
    let other = h.shell(&second)?;
    h.layout(&first, std::slice::from_ref(&agent))?;
    h.layout(&second, std::slice::from_ref(&other))?;
    h.rpc(json!({"SelectProject":{"project":id(&second)}}))?;
    save_prefs(
        &h,
        &json!({"version":1,"attention_migrated":true,"tool":"Git","visible":true}),
    )?;
    let event = json!({"protocol_version":1,"event_id":"bell-waiting","terminal_session_id":id(&agent),"agent_invocation_id":"bell-agent","agent_kind":"custom","provider_session_id":"bell-provider","state":"waiting_permission","request_id":"bell-request","sequence":1,"summary":"Agent needs permission","details":"Isolated bell fixture","resume":null});
    h.rpc(json!({"Hook":event}))?;
    Ok(Fixture {
        h,
        first,
        second,
        agent,
        other,
        event,
    })
}

impl Fixture {
    /// Wait for the GUI snapshot to satisfy `check`.
    #[track_caller]
    fn wait_gui(&self, check: impl Fn(&Value) -> bool) -> Result<()> {
        self.h.wait(|_| gui(&self.h).is_ok_and(|s| check(&s)), 8)?;
        Ok(())
    }
    fn waiting_projects(&self, o: &Options) -> Result<()> {
        capture(
            CaptureInput {
                h: &self.h,
                opts: o,
                name: "waiting-projects",
                actions: json!([]),
                after: 2000,
            },
            |_| {
                self.wait_gui(|s| {
                    s.get("left_agents")
                        .is_some_and(|value| value == &json!(false))
                        && s.get("agent_bar_badge")
                            .is_some_and(|badge| badge == "1 waiting · 1 unread")
                        && s.get("attention")
                            .is_some_and(|attention| attention == &json!([1, false]))
                })
            },
        )?;
        Ok(())
    }
    fn bell_open_on_git(&self, o: &Options) -> Result<()> {
        capture(
            CaptureInput {
                h: &self.h,
                opts: o,
                name: "bell-open-on-git",
                actions: json!([{"at_ms":900,"target":"attention-bell"}]),
                after: 2300,
            },
            |_| {
                self.wait_gui(|s| {
                    s.get("attention")
                        .is_some_and(|attention| attention == &json!([1, true]))
                        && s.get("left_agents")
                            .is_some_and(|value| value == &json!(false))
                })
            },
        )?;
        Ok(())
    }
    // Cold start under parallel software rendering can push the first inbox
    // paint past a tight window; the click below only dispatches on a fresh
    // target rect, so allow room for paint, click, and settle.
    fn agent_navigation(&self, o: &Options) -> Result<()> {
        let Self {
            h, first, agent, ..
        } = self;
        capture(
            CaptureInput {
                h,
                opts: o,
                name: "agent-navigation",
                actions: json!([{"at_ms":1500,"target":format!("agent-go:{}",id(agent))}]),
                after: 4500,
            },
            |_| {
                self.wait_gui(|s| {
                    s.get("left_agents")
                        .is_some_and(|value| value == &json!(true))
                        && s.get("selected_project")
                            .is_some_and(|project| first.get("id").is_some_and(|id| project == id))
                        && s.get("active_session")
                            .is_some_and(|session| agent.get("id").is_some_and(|id| session == id))
                })
            },
        )?;
        // Navigation must not clear the observed waiting state.
        h.wait(
            |s| {
                s.get("agents")
                    .and_then(|agents| agents.get(0))
                    .and_then(|agent| agent.get("state"))
                    .is_some_and(|state| state == "waiting_permission")
            },
            5,
        )?;
        Ok(())
    }
    fn running(&self, o: &Options) -> Result<()> {
        let mut running = self.event.clone();
        let running_object = running
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("hook event is not an object"))?;
        running_object.insert("event_id".into(), json!("bell-running"));
        running_object.insert("state".into(), json!("running"));
        running_object.insert("sequence".into(), json!(2));
        capture(
            CaptureInput {
                h: &self.h,
                opts: o,
                name: "running",
                actions: json!([]),
                after: 3000,
            },
            |_| {
                self.wait_gui(|s| {
                    s.get("agent_bar_badge")
                        .and_then(Value::as_str)
                        .is_some_and(|badge| badge.contains("waiting"))
                })?;
                self.h.rpc(json!({"Hook":running}))?;
                self.wait_gui(|s| {
                    s.get("left_agents")
                        .is_some_and(|value| value == &json!(false))
                        && !s
                            .get("agent_bar_badge")
                            .and_then(Value::as_str)
                            .unwrap_or("waiting")
                            .contains("waiting")
                })
            },
        )?;
        Ok(())
    }
}
