use super::super::harness::{Harness, id};
use super::dispatch::CaptureInput;
use super::dispatch::{Options, capture};
use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::{fs, thread, time::Duration};
pub(crate) fn scrolling(opts: &Options) -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let project = h.project("scrolling-sample")?;
    let left = h.shell(&project)?;
    let right = h.shell(&project)?;
    h.layout(&project, &[left.clone(), right.clone()])?;
    for session in [&left, &right] {
        let mut stream = h.attach(session)?;
        h.write(&mut stream, "i=0; while [ $i -lt 500 ]; do printf 'Sample retained line %s\\n' \"$i\"; i=$((i+1)); done\n")?;
    }
    thread::sleep(Duration::from_millis(500));
    let left_target = format!("terminal:{}", id(&left));
    let right_target = format!("terminal:{}", id(&right));
    let logs = capture(
        CaptureInput {
            h: &h,
            opts,
            name: "hover-history",
            actions: json!([
                {"at_ms":900,"target":left_target},
                {"at_ms":1300,"target":right_target,"hover":true},
                {"at_ms":1500,"target":right_target,"scroll":4.0,"wheel_phase":"start"},
                {"at_ms":1600,"target":right_target,"scroll":4.0},
                {"at_ms":1700,"target":right_target,"scroll":4.0},
                {"at_ms":1800,"target":right_target,"scroll":100.0},
                {"at_ms":2200,"target":right_target,"scroll":2.0,"wheel_unit":"line"},
                {"at_ms":2400,"target":right_target,"scroll":0.0,"wheel_phase":"cancel"},
                {"at_ms":3700,"target":right_target,"scroll":-1000.0,"wheel_unit":"line"}
            ]),
            after: 4500,
        },
        |_| {
            thread::sleep(Duration::from_millis(2800));
            h.write(
                &mut h.attach(&right)?,
                "printf 'Sample arriving output\\n'\n",
            )?;
            Ok(())
        },
    )?;
    fs::create_dir_all(&opts.output)?;
    fs::write(opts.output.join("scroll-evidence.log"), &logs)?;
    let prefix = format!(
        "Scroll evidence: session={} focused=false offset=",
        id(&right)
    );
    ensure!(
        logs.lines()
            .filter_map(|l| l.strip_prefix(&prefix))
            .any(|s| s
                .split_whitespace()
                .next()
                .and_then(|n| n.parse::<usize>().ok())
                .is_some_and(|n| n > 0)),
        "Hover-only scrolling did not move unfocused history: {logs}"
    );
    let offsets: Vec<usize> = logs
        .lines()
        .filter_map(|l| l.strip_prefix(&prefix))
        .filter_map(|s| s.split_whitespace().next()?.parse().ok())
        .collect();
    ensure!(
        offsets.last() == Some(&0),
        "Scrolling down did not return to recent output: {logs}"
    );
    ensure!(
        offsets.iter().filter(|offset| **offset > 0).count() >= 2,
        "Missing retained-history observations during output"
    );
    let left_prefix = format!(
        "Scroll evidence: session={} focused=true offset=",
        id(&left)
    );
    ensure!(
        !logs
            .lines()
            .filter_map(|l| l.strip_prefix(&left_prefix))
            .any(|s| !s.starts_with("0 ")),
        "Focused pane scrolled with hovered pane: {logs}"
    );
    h.assert_pids(&[left, right])?;
    Ok(())
}

/// Muse-like primary-screen output must still scroll after hiding and restoring
/// its tab. The producer does not redraw on SIGWINCH, so resize cannot mask loss.
pub(crate) fn scrolling_bounded(opts: &Options) -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let project = h.project("bounded-scroll-sample")?;
    let sample = h.shell(&project)?;
    let other = h.shell(&project)?;
    h.rpc(json!({"Rename":{"session":id(&sample),"label":"Muse sample"}}))?;
    h.rpc(json!({"Rename":{"session":id(&other),"label":"Other"}}))?;
    let first = h.layout(&project, std::slice::from_ref(&sample))?;
    let second = h.layout(&project, std::slice::from_ref(&other))?;
    h.rpc(json!({"SaveLayout":{"project":id(&project),"layout":{
        "version":2,"active":"sample","tabs":[
            {"id":"sample","primary":{"Terminal":id(&sample)},"layout":first},
            {"id":"other","primary":{"Terminal":id(&other)},"layout":second}
        ]
    }}}))?;
    h.write(
        &mut h.attach(&sample)?,
        "printf '\\033[2J\\033[H\\033[20;1HFIXED FOOTER\\033[1;19r\\033[H'; i=0; while [ $i -lt 80 ]; do printf 'PUBLIC BOUNDED SAMPLE %03d\\r\\n' \"$i\"; i=$((i+1)); done; printf '\\033[r'\n",
    )?;
    thread::sleep(Duration::from_millis(500));
    let target = format!("terminal:{}", id(&sample));
    let logs = capture(
        CaptureInput {
            h: &h,
            opts,
            name: "bounded-reconnect",
            actions: json!([
                {"at_ms":1000,"target":target,"scroll":8.0,"wheel_unit":"line"},
                {"at_ms":1600,"target":"workspace-tab:Other"},
                {"at_ms":2200,"target":"workspace-tab:Muse sample"},
                {"at_ms":2900,"target":target,"scroll":8.0,"wheel_unit":"line"},
                {"at_ms":3400,"target":target,"scroll":-1000.0,"wheel_unit":"line"}
            ]),
            after: 4000,
        },
        |_| Ok(()),
    )?;
    let prefix = format!("Scroll evidence: session={} ", id(&sample));
    let offsets = |text: &str| -> Vec<usize> {
        text.lines()
            .filter(|line| line.starts_with(&prefix))
            .filter_map(|line| {
                line.split(" offset=")
                    .nth(1)?
                    .split_whitespace()
                    .next()?
                    .parse()
                    .ok()
            })
            .collect()
    };
    // A wheel sent before the first terminal paint settles can be dropped
    // without a trace. Anchor on the tab return: retained history must scroll
    // after the restore, and scrolling down must return to recent output.
    let restored = logs
        .split("Fixture action: workspace-tab:Muse sample")
        .nth(1)
        .unwrap_or_default();
    let restored_offsets = offsets(restored);
    ensure!(
        restored_offsets.iter().any(|offset| *offset > 0) && restored_offsets.last() == Some(&0),
        "Bounded-region scrollback failed after tab reconnect: {restored_offsets:?}; {logs}"
    );
    h.assert_pids(&[sample, other])?;
    Ok(())
}

/// Shared driver for the agent-wheel fixtures. The fake agent switches to
/// the alternate screen, enables `init` mouse modes, then records every
/// input byte it receives. `start_before_attach` covers the reattach
/// snapshot path; otherwise the agent starts while the GUI is attached
/// (live-bytes path). Pass the SGR (or legacy) report markers to expect.
pub(crate) struct AgentWheelCaseInput<'a> {
    pub(crate) opts: &'a Options,
    pub(crate) name: &'a str,
    pub(crate) init: &'a str,
    pub(crate) up_marker: &'a str,
    pub(crate) down_marker: &'a str,
    pub(crate) start_before_attach: bool,
    pub(crate) wheel_at_ms: u64,
    pub(crate) after_ms: u64,
}

pub(crate) fn agent_wheel_case(input: AgentWheelCaseInput<'_>) -> Result<()> {
    let AgentWheelCaseInput {
        opts,
        name,
        init,
        up_marker,
        down_marker,
        start_before_attach,
        wheel_at_ms,
        after_ms,
    } = input;
    let h = Harness::new()?;
    h.setup()?;
    let project = h.project(&format!("{name}-sample"))?;
    let session = h.shell(&project)?;
    h.layout(&project, std::slice::from_ref(&session))?;
    let sid = id(&session).to_string();
    let log = std::env::temp_dir().join(format!(
        "terminator-{name}-{}-{}.bin",
        std::process::id(),
        sid.chars().take(8).collect::<String>()
    ));
    let _ = fs::remove_file(&log);
    let command = format!(
        "stty raw -echo; printf '{init}\\033[Hagent-ready\\n'; dd bs=1 count=512 > {} 2>/dev/null\n",
        log.display()
    );
    if start_before_attach {
        let mut stream = h.attach(&session)?;
        h.write(&mut stream, &command)?;
        thread::sleep(Duration::from_secs(1));
    }
    let target = format!("terminal:{sid}");
    let wheel = |at_ms: u64, delta: f64, phase: &str| serde_json::json!({"at_ms":at_ms,"target":target,"scroll":delta,"wheel_unit":"line","wheel_phase":phase});
    let logs = capture(
        CaptureInput {
            h: &h,
            opts,
            name,
            actions: json!([
                {"at_ms":900,"target":target},
                {"at_ms":1300,"target":target,"hover":true},
                wheel(wheel_at_ms, 2.0, "start"),
                wheel(wheel_at_ms.saturating_add(200), 2.0, "move"),
                wheel(wheel_at_ms.saturating_add(400), -1.0, "move"),
                wheel(wheel_at_ms.saturating_add(600), 0.0, "cancel"),
            ]),
            after: after_ms,
        },
        |_| {
            if !start_before_attach {
                thread::sleep(Duration::from_millis(500));
                h.write(&mut h.attach(&session)?, &command)?;
            }
            Ok(())
        },
    )?;
    let bytes = fs::read(&log).with_context(|| format!("missing agent log {log:?}; {logs}"))?;
    let text = String::from_utf8_lossy(&bytes);
    ensure!(
        text.contains(up_marker),
        "Agent never received wheel-up reports: {text:?}; {logs}"
    );
    ensure!(
        text.contains(down_marker),
        "Agent never received wheel-down reports: {text:?}; {logs}"
    );
    let prefix = format!("Scroll evidence: session={sid} focused=true offset=");
    ensure!(
        logs.lines().find_map(|l| l.strip_prefix(&prefix)).is_some(),
        "Missing scroll evidence for agent session: {logs}"
    );
    h.assert_pids(&[session])?;
    let _ = fs::remove_file(&log);
    Ok(())
}

/// Wheel input over a full-screen SGR-mouse agent must reach the agent as
/// mouse reports, not local scrollback. The fake agent enables the same
/// modes as real agent TUIs (alt-screen plus SGR mouse) before the GUI
/// attaches, so this also covers the reattach snapshot path.
pub(crate) fn agent_wheel(opts: &Options) -> Result<()> {
    agent_wheel_case(AgentWheelCaseInput {
        opts,
        name: "agent-wheel",
        init: "\\033[?1049h\\033[?1000h\\033[?1006h",
        up_marker: "[<64;",
        down_marker: "[<65;",
        start_before_attach: true,
        wheel_at_ms: 1500,
        after_ms: 4500,
    })
}

/// A legacy (non-SGR) mouse agent must receive `ESC [ M` reports with the
/// single-byte coordinates instead of local scrollback or arrow keys.
pub(crate) fn agent_wheel_legacy(opts: &Options) -> Result<()> {
    agent_wheel_case(AgentWheelCaseInput {
        opts,
        name: "agent-wheel-legacy",
        init: "\\033[?1049h\\033[?1000h",
        up_marker: "\u{1b}[M`",
        down_marker: "\u{1b}[Ma",
        start_before_attach: true,
        wheel_at_ms: 1500,
        after_ms: 4500,
    })
}

/// An agent that starts while the GUI is already attached exercises the
/// live-bytes path instead of the reattach snapshot.
pub(crate) fn agent_wheel_live(opts: &Options) -> Result<()> {
    agent_wheel_case(AgentWheelCaseInput {
        opts,
        name: "agent-wheel-live",
        init: "\\033[?1049h\\033[?1000h\\033[?1006h",
        up_marker: "[<64;",
        down_marker: "[<65;",
        start_before_attach: false,
        wheel_at_ms: 3000,
        after_ms: 6500,
    })
}
