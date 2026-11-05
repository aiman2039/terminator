//! Commit (also amend) and push. Hooks and the network may need minutes, so
//! these calls do not get the quick-call cap: they run in the async process
//! pool with a long safety cap, and Cancel sends their process group Ctrl-C
//! (then kills what still runs).
use super::Report;
use anyhow::ensure;
use std::{iter::Peekable, path::Path, process::Output, str::Chars, time::Duration};
use terminator_core::{
    CommandOptions,
    async_process::{AsyncProcessError, Processes},
    async_service::CancellationToken,
    git,
};

/// Safety cap for one `git commit` or `git push`.
pub(super) const LONG_GIT_TIMEOUT: Duration = Duration::from_mins(30);
const LONG_GIT_LIMIT: &str = "30 minutes";
/// Output kept from one call, so a verbose hook cannot fail the commit.
const OUTPUT_LIMIT: usize = 8 * 1024 * 1024;
/// A failure shows at most this many of the last output lines and characters.
const TAIL_LINES: usize = 40;
const TAIL_CHARS: usize = 4000;

/// Error prefix for a push that failed after its commit landed; the draft
/// must not be restored then, or committing again would duplicate the commit.
pub(crate) const PUSH_FAILED: &str = "Committed, but push failed";
/// Cancel stopped the commit: nothing was committed.
pub(crate) const COMMIT_CANCELLED: &str = "Commit cancelled";
/// Cancel stopped the push; the commit itself landed.
pub(crate) const PUSH_CANCELLED: &str = "Committed, but push was cancelled";
/// Cancel came after Git made the commit (a `post-commit` hook still ran):
/// the commit landed, and no push follows.
pub(crate) const COMMITTED_CANCELLED: &str = "Committed; Cancel stopped its hooks";

/// The commit itself landed: only its push failed or was cancelled.
pub(crate) fn commit_landed(error: &str) -> bool {
    error.starts_with(PUSH_FAILED)
        || error.starts_with(PUSH_CANCELLED)
        || error == COMMITTED_CANCELLED
}

/// The user stopped the job: a notice, not an error.
pub(crate) fn commit_cancelled(error: &str) -> bool {
    [COMMIT_CANCELLED, PUSH_CANCELLED, COMMITTED_CANCELLED].contains(&error)
}

/// A push that did not run or report after its commit landed.
pub(crate) fn push_failed(reason: &dyn std::fmt::Display) -> String {
    format!("{PUSH_FAILED}: {reason}")
}

/// Status of an action that a running commit refused.
pub(crate) const COMMIT_RUNNING: &str =
    "A commit is running in this repository. Wait for it to finish.";
/// Hover text of a control that a running commit disables.
pub(crate) const COMMIT_WAIT: &str = "Wait for the commit to finish";

/// What an action can change in a commit that runs in its repository.
/// `git commit` reads the index again after `pre-commit` and moves `HEAD`
/// only at its end, so until then these writes change the commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommitRisk {
    /// Nothing: reads, Explorer file operations, Perforce.
    None,
    /// One worktree's index or files: staging, Discard.
    Index,
    /// The index and the stash. Every worktree of a repository shares the
    /// stash, and a hook can keep its own entry there: Shelve, Unshelve,
    /// Drop.
    Stash,
    /// One worktree's `HEAD`: Switch. The push after the commit needs it
    /// too: `git push` with no refspec reads `HEAD` when it starts.
    Head,
}

/// The message and options of one commit.
#[derive(Clone, Copy)]
pub(crate) struct CommitRequest<'a> {
    pub(crate) message: &'a str,
    pub(crate) amend: bool,
    pub(crate) signoff: bool,
}

/// Runs a commit job's Git calls in `root`. Cancelling `cancel` stops the
/// running call's process group, hooks included.
pub(crate) struct CommitRun<'a> {
    pub(crate) processes: &'a Processes,
    pub(crate) root: &'a Path,
    pub(crate) cancel: &'a CancellationToken,
}

/// Why a long Git call did not succeed.
enum Stop {
    Cancelled,
    /// Follows the action's name: ` (exit status: 1)` and the output tail.
    Failed(String),
}

impl CommitRun<'_> {
    /// `git commit`; `Ok` once the commit landed. Git moves `HEAD` before
    /// `post-commit` runs, so a Cancel that stops a later hook still
    /// reports the commit ([`COMMITTED_CANCELLED`]).
    pub(crate) async fn commit(&self, request: CommitRequest<'_>) -> Result<(), String> {
        let (args, input) = commit_args(request).map_err(|error| format!("{error:#}"))?;
        let before = self.head().await;
        match self.git(&args, input).await {
            Ok(()) => Ok(()),
            Err(Stop::Cancelled) if self.head().await != before => Err(COMMITTED_CANCELLED.into()),
            Err(Stop::Cancelled) => Err(COMMIT_CANCELLED.into()),
            Err(Stop::Failed(detail)) => Err(format!("Commit failed{detail}")),
        }
    }

    /// The commit `HEAD` names, if any; read with the quick-call cap.
    async fn head(&self) -> Option<Vec<u8>> {
        let command = git::command(self.root, &["rev-parse", "--verify", "-q", "HEAD"]);
        let output = self
            .processes
            .run(command, CommandOptions::default(), None)
            .await
            .ok()?;
        output.status.success().then_some(output.stdout)
    }

    /// `git push`. Its errors say that the commit already landed.
    pub(crate) async fn push(&self) -> Result<Report, String> {
        match self.git(&["push"], None).await {
            Ok(()) => Ok(Report::Message("Committed and pushed".into())),
            Err(Stop::Cancelled) => Err(PUSH_CANCELLED.into()),
            Err(Stop::Failed(detail)) => Err(format!("{PUSH_FAILED}{detail}")),
        }
    }

    async fn git(&self, args: &[&str], input: Option<Vec<u8>>) -> Result<(), Stop> {
        let mut command = git::command(self.root, args);
        // A GUI has no terminal to answer a password prompt: fail at once.
        command.env("GIT_TERMINAL_PROMPT", "0");
        let result = self
            .processes
            .run_cancellable(command, long_options(input), self.cancel)
            .await;
        match result {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) => Err(Stop::Failed(failure(&output))),
            Err(_) if self.cancel.is_cancelled() => Err(Stop::Cancelled),
            Err(AsyncProcessError::TimedOut { .. }) => {
                Err(Stop::Failed(format!(": stopped after {LONG_GIT_LIMIT}")))
            }
            Err(error) => Err(Stop::Failed(format!(": {error}"))),
        }
    }
}

/// Commit and push options: the long cap, room for verbose hooks, and any
/// exit status, because the failure is read from the output.
pub(super) fn long_options(input: Option<Vec<u8>>) -> CommandOptions {
    CommandOptions {
        timeout: LONG_GIT_TIMEOUT,
        input,
        stdout_limit: OUTPUT_LIMIT,
        stderr_limit: OUTPUT_LIMIT,
        accepted_exit_codes: None,
    }
}

/// `git commit` arguments and the message for stdin. Amending with no
/// message keeps the previous message untouched.
fn commit_args(request: CommitRequest<'_>) -> anyhow::Result<(Vec<&'static str>, Option<Vec<u8>>)> {
    let message = request.message.trim();
    ensure!(
        request.amend || !message.is_empty(),
        "Commit message is empty"
    );
    ensure!(
        !message.contains('\0'),
        "Commit message contains a null byte"
    );
    let mut args = vec!["commit"];
    if request.amend {
        args.push("--amend");
    }
    if request.signoff {
        args.push("--signoff");
    }
    if request.amend && message.is_empty() {
        args.push("--no-edit");
        return Ok((args, None));
    }
    args.extend(["-F", "-"]);
    Ok((args, Some(message.as_bytes().to_vec())))
}

/// ` (exit status: 1)` and the end of the output. Git puts hook output on
/// stderr; its own refusals ("nothing to commit") may use stdout only.
fn failure(output: &Output) -> String {
    let stream = if output.stderr.iter().all(u8::is_ascii_whitespace) {
        &output.stdout
    } else {
        &output.stderr
    };
    let tail = tail(&String::from_utf8_lossy(stream));
    if tail.is_empty() {
        format!(" ({})", output.status)
    } else {
        format!(" ({})\n{tail}", output.status)
    }
}

/// The end of terminal output as plain text: at most [`TAIL_LINES`] lines
/// and [`TAIL_CHARS`] characters. A first line `…` marks a cut.
pub(super) fn tail(text: &str) -> String {
    let plain = plain(text);
    let lines: Vec<&str> = plain.lines().map(str::trim_end).collect();
    let end = lines
        .iter()
        .rposition(|line| !line.is_empty())
        .map_or(0, |last| last.saturating_add(1));
    let mut kept: Vec<String> = Vec::new();
    let mut budget = TAIL_CHARS;
    let mut cut = false;
    for line in lines.get(..end).unwrap_or_default().iter().rev() {
        let count = line.chars().count();
        if kept.len() == TAIL_LINES || count > budget {
            if kept.is_empty() {
                kept.push(last_chars(line, budget));
            }
            cut = true;
            break;
        }
        budget = budget.saturating_sub(count.saturating_add(1));
        kept.push((*line).to_owned());
    }
    // `kept` runs backwards: its last entries are the first lines.
    while kept.last().is_some_and(String::is_empty) {
        kept.pop();
    }
    if cut {
        kept.push("…".into());
    }
    kept.reverse();
    kept.join("\n")
}

fn last_chars(line: &str, count: usize) -> String {
    let skip = line.chars().count().saturating_sub(count);
    line.chars().skip(skip).collect()
}

/// Terminal output as plain text: escape sequences and other control
/// characters are dropped, and a carriage return starts its line again,
/// the way progress meters redraw.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => skip_escape(&mut chars),
            '\r' if chars.peek() != Some(&'\n') => {
                let start = out
                    .rfind('\n')
                    .map_or(0, |newline| newline.saturating_add(1));
                out.truncate(start);
            }
            '\n' | '\t' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Skips one escape sequence after its ESC: CSI up to its final byte, OSC
/// up to BEL or ST, or a single character.
fn skip_escape(chars: &mut Peekable<Chars<'_>>) {
    match chars.next() {
        Some('[') => {
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
        }
        Some(']') => {
            while let Some(c) = chars.next() {
                if c == '\u{7}' {
                    break;
                }
                if c == '\u{1b}' {
                    chars.next();
                    break;
                }
            }
        }
        _ => {}
    }
}
