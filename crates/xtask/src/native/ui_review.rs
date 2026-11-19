//! Screens of the UI review changes, for visual checks: the narrow
//! Settings section menu, the active workspace tab, an Agents inbox card
//! with its Peek action, Find in Files with a folded filter chip, the
//! Close tab dialog with session names, and the Remove worktrees list.
//! Run with `--narrow` for the narrow Settings menu.
use super::{CaptureInput, Options, Result, Value, git, id, json, output, plain, setup};

pub fn run(o: &Options) -> Result<()> {
    let (h, project, sessions, root) = setup("ui-review")?;
    h.update_settings(|settings| {
        settings.insert("dismissal".into(), json!("Manual"));
        settings.insert("notifications_side".into(), json!(false));
    })?;
    let agent = sessions.first().map(id).unwrap_or_default().to_owned();
    let event = json!({"protocol_version":1,"event_id":"review-waiting","terminal_session_id":agent,"agent_invocation_id":"review-agent","agent_kind":"custom","provider_session_id":"review-provider","state":"waiting_permission","request_id":"review-request","sequence":1,"summary":"Agent needs permission to run cargo test","details":"UI review fixture","resume":null});
    h.rpc(json!({"Hook":event}))?;
    let shot = |name: &str, actions: Value, after: u64| {
        plain(CaptureInput {
            h: &h,
            opts: o,
            name,
            actions,
            after,
        })
    };
    shot("tabs", json!([]), 2000)?;
    // Only a narrow card has the Section menu.
    if o.narrow {
        shot(
            "settings-menu",
            json!([{"at_ms":900,"target":"stripe-tool-Git"},{"at_ms":1600,"target":"settings"},{"at_ms":2200,"target":"settings-section-menu"}]),
            3200,
        )?;
    }
    shot(
        "close-tab",
        json!([{"at_ms":1100,"target":"workspace-close:Terminal 1"}]),
        2500,
    )?;
    shot(
        "find-files",
        json!([
            {"at_ms":1000,"target":"workspace-tab:Terminal 1","key":"F","command":true,"shift":true},
            {"at_ms":1800,"target":"find-files-filters"},
            {"at_ms":2300,"target":"find-files-mask"},
            {"at_ms":2600,"target":"find-files-mask","text":"*.rs, src/**"},
            {"at_ms":3000,"target":"find-files-exclude"},
            {"at_ms":3300,"target":"find-files-exclude","text":"target/**"},
            {"at_ms":3700,"target":"find-files-filters"}
        ]),
        4600,
    )?;
    worktrees(&h, o, &project, &root)?;
    // Last: opening the Agents inbox stays in the saved sidebar choice.
    shot(
        "inbox-card",
        json!([{"at_ms":900,"target":"left-agent-bar"},{"at_ms":1700,"target":format!("agent-row:{agent}"),"hover":true}]),
        2800,
    )?;
    Ok(())
}

/// Two task worktrees of the project's repository, ticked for Remove.
fn worktrees(
    h: &super::Harness,
    o: &Options,
    project: &Value,
    root: &std::path::Path,
) -> Result<()> {
    git(root, &["init", "-q"])?;
    git(root, &["config", "user.name", "Fixture"])?;
    git(root, &["config", "user.email", "fixture@example.invalid"])?;
    std::fs::write(root.join("file"), "base\n")?;
    git(root, &["add", "."])?;
    git(root, &["commit", "-qm", "base"])?;
    let mut ticks = Vec::new();
    for (index, branch) in ["fix-login", "docs-update"].into_iter().enumerate() {
        let destination = h.root.join(format!("task-{index}"));
        let mut command = h.command("terminator-hook");
        command
            .args(["ctl", "worktree", "add", id(project)])
            .arg(&destination)
            .args(["--branch", branch]);
        output(command)?;
        let state = h.state()?;
        let path = destination.to_string_lossy();
        if let Some(task) = state
            .get("projects")
            .and_then(Value::as_array)
            .and_then(|projects| {
                projects
                    .iter()
                    .find(|p| p.get("path").and_then(Value::as_str) == Some(path.as_ref()))
            })
        {
            ticks.push(format!("worktree-tick:{}", id(task)));
        }
    }
    let mut actions = vec![json!({"at_ms":1000,"target":"worktree-add"})];
    let mut at = 1800u64;
    for tick in ticks {
        actions.push(json!({"at_ms":at,"target":tick}));
        at = at.saturating_add(400);
    }
    actions.push(json!({"at_ms":at,"target":"worktree-remove-selected"}));
    plain(CaptureInput {
        h,
        opts: o,
        name: "remove-worktrees",
        actions: Value::Array(actions),
        after: at.saturating_add(1200),
    })?;
    Ok(())
}
