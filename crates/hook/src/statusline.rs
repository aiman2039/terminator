//! `terminator-hook statusline --data-dir DIR`: Claude Code's status-line
//! command when the opt-in Usage source is installed. It records only the
//! `rate_limits` windows and, inside an app-created terminal, sends the
//! session's current model to the daemon as usage only (never lifecycle)
//! when it changed. Then it runs the user's own status-line command with
//! the same input and passes its output and exit code through. A failed
//! record or send never changes the user's status line.
use anyhow::Result;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
use terminator_integrations::statusline;

use crate::hook_event::HookCommand;

/// Largest status-line input read.
const INPUT_LIMIT: u64 = 1024 * 1024;
/// Longest wait for a model send after the user's command ended.
const SEND_WAIT: Duration = Duration::from_millis(1500);

pub fn run(args: &[String]) -> Result<()> {
    let data = args
        .iter()
        .position(|arg| arg == "--data-dir")
        .and_then(|at| args.get(at.saturating_add(1)))
        .map(PathBuf::from);
    let mut input = Vec::new();
    std::io::stdin().take(INPUT_LIMIT).read_to_end(&mut input)?;
    if let Some(data) = &data
        && let Some(record) = statusline::quota_record(&input, terminator_core::now())
    {
        let path = terminator_core::agents::quota::claude_quota_path(data);
        let _ = terminator_core::atomic_write(&path, &record);
    }
    let started = Instant::now();
    let send = data
        .as_deref()
        .and_then(|data| send_model(args, data, &input));
    let status = data
        .as_deref()
        .and_then(statusline::original_command)
        .map(|command| run_original(&command, &input))
        .transpose()?;
    if let Some(done) = send {
        let _ = done.recv_timeout(SEND_WAIT.saturating_sub(started.elapsed()));
    }
    if let Some(status) = status {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

fn run_original(command: &str, input: &[u8]) -> Result<std::process::ExitStatus> {
    let mut child = Command::new("sh")
        .args(["-c", command])
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input);
    }
    Ok(child.wait()?)
}

/// Starts sending the current model in the background when it differs from
/// the last one sent for this terminal and session. Only inside an
/// app-created terminal; `None` when nothing is sent.
fn send_model(args: &[String], data: &Path, input: &[u8]) -> Option<mpsc::Receiver<()>> {
    let terminal = std::env::var("TERMINATOR_SESSION_ID").ok()?;
    let payload: serde_json::Value = serde_json::from_slice(input).ok()?;
    let usage = statusline::usage(&payload)?;
    let model = usage.model.clone()?;
    let provider = payload.get("session_id")?.as_str()?;
    let key = format!("{terminal}:{provider}");
    if !statusline::SentModels::load(data).due(&key, &model, terminator_core::now()) {
        return None;
    }
    let (tx, rx) = mpsc::channel();
    let args = args.to_vec();
    let data = data.to_path_buf();
    std::thread::spawn(move || {
        let delivered =
            HookCommand::deliver_usage(&args, "claude", &payload, usage).unwrap_or(false);
        // Read again: another status-line run may have saved meanwhile.
        let mut sent = statusline::SentModels::load(&data);
        let at = terminator_core::now();
        sent.record(&key, &model, statusline::Attempt { delivered, at });
        let _ = sent.save(&data);
        let _ = tx.send(());
    });
    Some(rx)
}
