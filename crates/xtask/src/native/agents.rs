use super::*;
use terminator_core::{Paths, ui_control};

fn gui(h: &Harness) -> Result<Value> {
    ui_control::rpc(&Paths::at(h.root.clone()), ui_control::Request::Snapshot)
}

pub fn run(o: &Options) -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let first = h.project("agent-project")?;
    let second = h.project("other-project")?;
    let agent = h.shell(&first)?;
    let other = h.shell(&second)?;
    h.layout(&first, std::slice::from_ref(&agent))?;
    h.layout(&second, std::slice::from_ref(&other))?;
    h.rpc(json!({"SelectProject":{"project":id(&second)}}))?;
    save_prefs(
        &h,
        &json!({"version":1,"all_projects":true,"attention_migrated":true}),
    )?;
    let event = json!({"protocol_version":1,"event_id":"bell-waiting","terminal_session_id":id(&agent),"agent_invocation_id":"bell-agent","agent_kind":"custom","provider_session_id":"bell-provider","state":"waiting_permission","request_id":"bell-request","sequence":1,"summary":"Agent needs permission","details":"Isolated bell fixture","resume":null});
    h.rpc(json!({"Hook":event}))?;
    capture(&h, o, "waiting-projects", json!([]), 2000, |_| {
        h.wait(
            |_| {
                gui(&h).is_ok_and(|s| {
                    s["left_agents"] == false && s["agent_bar_badge"] == "1 waiting · 1 unread"
                })
            },
            8,
        )?;
        Ok(())
    })?;
    capture(
        &h,
        o,
        "agents-open",
        json!([{"at_ms":900,"target":"left-agent-bar"}]),
        2300,
        |_| {
            h.wait(
                |_| {
                    gui(&h).is_ok_and(|s| {
                        s["left_agents"] == true && s["selected_project"] == second["id"]
                    })
                },
                8,
            )?;
            Ok(())
        },
    )?;
    ensure!(
        prefs(&h)?["left_agents"] == true,
        "Opening Agents did not persist"
    );
    capture(
        &h,
        o,
        "agent-navigation",
        json!([{"at_ms":1200,"target":format!("agent-row:{}",id(&agent))}]),
        2700,
        |_| {
            h.wait(
                |_| {
                    gui(&h).is_ok_and(|s| {
                        s["left_agents"] == true
                            && s["selected_project"] == first["id"]
                            && s["active_session"] == agent["id"]
                    })
                },
                8,
            )?;
            Ok(())
        },
    )?;
    // Focus may dismiss the notice, but must not clear the observed waiting state.
    h.wait(|s| s["agents"][0]["state"] == "waiting_permission", 5)?;
    capture(
        &h,
        o,
        "projects-restored",
        json!([{"at_ms":900,"target":"left-agent-bar"}]),
        2300,
        |_| {
            h.wait(
                |_| {
                    gui(&h).is_ok_and(|s| {
                        s["left_agents"] == false
                            && s["agent_bar_badge"]
                                .as_str()
                                .is_some_and(|s| s.starts_with("1 waiting"))
                    })
                },
                8,
            )?;
            Ok(())
        },
    )?;
    ensure!(
        prefs(&h)?["left_agents"] == false,
        "Returning to Projects did not persist"
    );
    let mut running = event.clone();
    running["event_id"] = json!("bell-running");
    running["state"] = json!("running");
    running["sequence"] = json!(2);
    capture(&h, o, "running", json!([]), 3000, |_| {
        h.wait(
            |_| {
                gui(&h).is_ok_and(|s| {
                    s["agent_bar_badge"]
                        .as_str()
                        .is_some_and(|s| s.contains("waiting"))
                })
            },
            8,
        )?;
        h.rpc(json!({"Hook":running}))?;
        h.wait(
            |_| {
                gui(&h).is_ok_and(|s| {
                    s["left_agents"] == false
                        && !s["agent_bar_badge"]
                            .as_str()
                            .unwrap_or("waiting")
                            .contains("waiting")
                })
            },
            8,
        )?;
        Ok(())
    })?;
    h.assert_pids(&[agent, other])
}
