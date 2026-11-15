//! `terminator-hook statusline --data-dir DIR`: Claude Code's status-line
//! command when the opt-in Usage source is installed. It records only the
//! `rate_limits` windows, then runs the user's own status-line command with
//! the same input and passes its output and exit code through. A failed
//! record never changes the user's status line.
use anyhow::Result;
use std::{
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
};
use terminator_integrations::statusline;

/// Largest status-line input read.
const INPUT_LIMIT: u64 = 1024 * 1024;

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
    let Some(command) = data.as_deref().and_then(statusline::original_command) else {
        return Ok(());
    };
    let mut child = Command::new("sh")
        .args(["-c", &command])
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(&input);
    }
    let status = child.wait()?;
    std::process::exit(status.code().unwrap_or(1));
}
