use super::super::harness::{Harness, id};
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
        &h,
        opts,
        "hover-history",
        json!([
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
        4500,
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

/// Shared driver for the agent-wheel fixtures. The fake agent switches to
/// the alternate screen, enables `init` mouse modes, then records every
/// input byte it receives. `start_before_attach` covers the reattach
/// snapshot path; otherwise the agent starts while the GUI is attached
/// (live-bytes path). Pass the SGR (or legacy) report markers to expect.
#[allow(clippy::too_many_arguments)]
pub(crate) fn agent_wheel_case(
    opts: &Options,
    name: &str,
    init: &str,
    up_marker: &str,
    down_marker: &str,
    start_before_attach: bool,
    wheel_at_ms: u64,
    after_ms: u64,
) -> Result<()> {
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
        &h,
        opts,
        name,
        json!([
            {"at_ms":900,"target":target},
            {"at_ms":1300,"target":target,"hover":true},
            wheel(wheel_at_ms, 2.0, "start"),
            wheel(wheel_at_ms.saturating_add(200), 2.0, "move"),
            wheel(wheel_at_ms.saturating_add(400), -1.0, "move"),
            wheel(wheel_at_ms.saturating_add(600), 0.0, "cancel"),
        ]),
        after_ms,
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
    agent_wheel_case(
        opts,
        "agent-wheel",
        "\\033[?1049h\\033[?1000h\\033[?1006h",
        "[<64;",
        "[<65;",
        true,
        1500,
        4500,
    )
}

/// A legacy (non-SGR) mouse agent must receive `ESC [ M` reports with the
/// single-byte coordinates instead of local scrollback or arrow keys.
pub(crate) fn agent_wheel_legacy(opts: &Options) -> Result<()> {
    agent_wheel_case(
        opts,
        "agent-wheel-legacy",
        "\\033[?1049h\\033[?1000h",
        "\u{1b}[M`",
        "\u{1b}[Ma",
        true,
        1500,
        4500,
    )
}

/// An agent that starts while the GUI is already attached exercises the
/// live-bytes path instead of the reattach snapshot.
pub(crate) fn agent_wheel_live(opts: &Options) -> Result<()> {
    agent_wheel_case(
        opts,
        "agent-wheel-live",
        "\\033[?1049h\\033[?1000h\\033[?1006h",
        "[<64;",
        "[<65;",
        false,
        3000,
        6500,
    )
}
