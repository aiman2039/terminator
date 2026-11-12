//! Read-only pull request data from `gh` (GitHub) or `glab` (GitLab),
//! picked by the `origin` host as Create PR does. Blocking: it runs on the
//! file pool, never on the GUI thread, and never prompts.
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use terminator_core::{CommandOptions, git, run_command};

/// Cap of one CLI call, as for the other quick Git panel calls.
const CALL_CAP: Duration = Duration::from_secs(20);
/// Review threads read for one pull request.
const THREAD_LIMIT: usize = 50;

/// What the panel shows for the current branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PrState {
    Loading,
    /// The branch has no pull request.
    NoPr,
    /// The CLI is missing or not signed in: a quiet hint, not an error.
    Hint(String),
    Ready(PrInfo),
    Failed(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PrInfo {
    pub number: u64,
    pub title: String,
    /// `OPEN`, `MERGED`, `CLOSED` (GitHub) or `opened`, `merged` … (GitLab).
    pub state: String,
    pub draft: bool,
    pub url: String,
    /// `APPROVED`, `CHANGES_REQUESTED`, `REVIEW_REQUIRED`, if known.
    pub review: Option<String>,
    pub checks: Checks,
    pub threads: Vec<Thread>,
    /// Why the review threads could not be read, if they could not.
    pub threads_error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Checks {
    pub passed: usize,
    pub failed: usize,
    pub pending: usize,
    /// Names of the failed checks.
    pub failing: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Thread {
    pub path: String,
    pub line: Option<u32>,
    pub author: String,
    /// The first line of the first comment.
    pub body: String,
    pub resolved: bool,
}

/// The CLI output of one call: stdout, or why it failed.
fn call(program: &Path, root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let mut command = Command::new(program);
    command
        .current_dir(root)
        .args(args)
        .env("GH_PROMPT_DISABLED", "1")
        .env("GLAB_NO_PROMPT", "1")
        .env("NO_COLOR", "1");
    let options = CommandOptions {
        timeout: CALL_CAP,
        stdout_limit: 4 * 1024 * 1024,
        accepted_exit_codes: None,
        ..Default::default()
    };
    let output = run_command(command, options).map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

/// A failed view call: no pull request, a hint, or an error.
fn failure(cli: &str, error: String) -> PrState {
    let lower = error.to_ascii_lowercase();
    if lower.contains("no pull requests found")
        || lower.contains("no merge request")
        || lower.contains("no open merge request")
    {
        PrState::NoPr
    } else if lower.contains("auth login") || lower.contains("not logged") || lower.contains("401")
    {
        PrState::Hint(format!(
            "Run `{cli} auth login` in a terminal to see the pull request here."
        ))
    } else {
        PrState::Failed(error.lines().next().unwrap_or("Failed").to_owned())
    }
}

/// The pull request of the branch checked out at `root`. `program` replaces
/// the CLI found on `PATH` (tests use a fake one).
pub(crate) fn fetch(root: &Path, program: Option<&Path>) -> PrState {
    let options = CommandOptions {
        timeout: CALL_CAP,
        ..Default::default()
    };
    let remote = run_command(
        git::command(root, &["remote", "get-url", "origin"]),
        options,
    )
    .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned());
    let Ok(remote) = remote else {
        return PrState::Hint("The pull request panel needs an origin remote.".into());
    };
    let cli = crate::workspace_ops::review_cli(&remote);
    let program = program
        .map(PathBuf::from)
        .or_else(|| terminator_core::find_executable(cli));
    let Some(program) = program.filter(|path| path.is_file()) else {
        let name = if cli == "glab" {
            "GitLab CLI"
        } else {
            "GitHub CLI"
        };
        return PrState::Hint(format!(
            "Install {cli} ({name}) to see the pull request here."
        ));
    };
    if cli == "glab" {
        gitlab(&program, root)
    } else {
        github(&program, root)
    }
}

fn github(program: &Path, root: &Path) -> PrState {
    let fields = "number,title,state,url,isDraft,reviewDecision,statusCheckRollup";
    let view = match call(program, root, &["pr", "view", "--json", fields]) {
        Ok(view) => view,
        Err(error) => return failure("gh", error),
    };
    let Ok(view) = serde_json::from_slice::<Value>(&view) else {
        return PrState::Failed("gh gave output that is not JSON".into());
    };
    let mut info = parse_gh_view(&view);
    match github_threads(program, root, &info) {
        Ok(threads) => info.threads = threads,
        Err(error) => info.threads_error = Some(error),
    }
    PrState::Ready(info)
}

/// `owner` and `repo` from `https://host/owner/repo/pull/N`.
fn owner_repo(url: &str) -> Option<(String, String)> {
    let parsed = url::Url::parse(url).ok()?;
    let mut parts = parsed.path_segments()?;
    Some((parts.next()?.to_owned(), parts.next()?.to_owned()))
}

const THREADS_QUERY: &str = "query($owner:String!,$name:String!,$number:Int!){repository(owner:$owner,name:$name){pullRequest(number:$number){reviewThreads(first:50){nodes{isResolved path line comments(first:1){nodes{author{login} body}}}}}}}";

fn github_threads(program: &Path, root: &Path, info: &PrInfo) -> Result<Vec<Thread>, String> {
    let (owner, repo) = owner_repo(&info.url).ok_or("The pull request URL has no repository")?;
    let args = [
        "api".to_owned(),
        "graphql".to_owned(),
        "-f".to_owned(),
        format!("owner={owner}"),
        "-f".to_owned(),
        format!("name={repo}"),
        "-F".to_owned(),
        format!("number={}", info.number),
        "-f".to_owned(),
        format!("query={THREADS_QUERY}"),
    ];
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = call(program, root, &args)?;
    let value: Value = serde_json::from_slice(&out).map_err(|error| error.to_string())?;
    Ok(parse_gh_threads(&value))
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn first_line(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .chars()
        .take(200)
        .collect()
}

pub(super) fn parse_gh_view(view: &Value) -> PrInfo {
    let mut checks = Checks::default();
    for check in view
        .get("statusCheckRollup")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        // A CheckRun has status and conclusion; a StatusContext has state.
        let conclusion = text(check, "conclusion");
        let state = text(check, "state");
        let status = text(check, "status");
        let outcome = if !state.is_empty() {
            state
        } else if status != "COMPLETED" {
            "PENDING".to_owned()
        } else {
            conclusion
        };
        match outcome.as_str() {
            "SUCCESS" | "NEUTRAL" | "SKIPPED" => checks.passed = checks.passed.saturating_add(1),
            "PENDING" | "EXPECTED" | "QUEUED" | "IN_PROGRESS" | "" => {
                checks.pending = checks.pending.saturating_add(1);
            }
            _ => {
                checks.failed = checks.failed.saturating_add(1);
                let name = text(check, "name");
                checks.failing.push(if name.is_empty() {
                    text(check, "context")
                } else {
                    name
                });
            }
        }
    }
    PrInfo {
        number: view
            .get("number")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        title: text(view, "title"),
        state: text(view, "state"),
        draft: view
            .get("isDraft")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        url: text(view, "url"),
        review: Some(text(view, "reviewDecision")).filter(|review| !review.is_empty()),
        checks,
        ..PrInfo::default()
    }
}

pub(super) fn parse_gh_threads(value: &Value) -> Vec<Thread> {
    let nodes = value
        .pointer("/data/repository/pullRequest/reviewThreads/nodes")
        .and_then(Value::as_array);
    nodes
        .into_iter()
        .flatten()
        .take(THREAD_LIMIT)
        .map(|node| {
            let comment = node.pointer("/comments/nodes/0");
            Thread {
                path: text(node, "path"),
                line: node
                    .get("line")
                    .and_then(Value::as_u64)
                    .and_then(|line| u32::try_from(line).ok()),
                author: comment
                    .and_then(|c| c.pointer("/author/login"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                body: comment
                    .map(|c| first_line(&text(c, "body")))
                    .unwrap_or_default(),
                resolved: node
                    .get("isResolved")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            }
        })
        .collect()
}

fn gitlab(program: &Path, root: &Path) -> PrState {
    let view = match call(program, root, &["mr", "view", "--output", "json"]) {
        Ok(view) => view,
        Err(error) => return failure("glab", error),
    };
    let Ok(view) = serde_json::from_slice::<Value>(&view) else {
        return PrState::Failed("glab gave output that is not JSON".into());
    };
    let mut info = parse_glab_view(&view);
    let endpoint = format!(
        "projects/:fullpath/merge_requests/{}/discussions?per_page={THREAD_LIMIT}",
        info.number
    );
    match call(program, root, &["api", &endpoint]) {
        Ok(out) => match serde_json::from_slice::<Value>(&out) {
            Ok(value) => info.threads = parse_glab_threads(&value),
            Err(error) => info.threads_error = Some(error.to_string()),
        },
        Err(error) => info.threads_error = Some(error),
    }
    PrState::Ready(info)
}

pub(super) fn parse_glab_view(view: &Value) -> PrInfo {
    let mut checks = Checks::default();
    match view
        .pointer("/head_pipeline/status")
        .and_then(Value::as_str)
    {
        Some("success" | "skipped" | "manual") => checks.passed = 1,
        Some("failed" | "canceled") => {
            checks.failed = 1;
            checks.failing.push("pipeline".into());
        }
        Some(_) => checks.pending = 1,
        None => {}
    }
    PrInfo {
        number: view.get("iid").and_then(Value::as_u64).unwrap_or_default(),
        title: text(view, "title"),
        state: text(view, "state"),
        draft: view.get("draft").and_then(Value::as_bool).unwrap_or(false),
        url: text(view, "web_url"),
        review: None,
        checks,
        ..PrInfo::default()
    }
}

pub(super) fn parse_glab_threads(value: &Value) -> Vec<Thread> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|discussion| {
            let note = discussion.pointer("/notes/0")?;
            note.get("resolvable")
                .and_then(Value::as_bool)
                .filter(|resolvable| *resolvable)?;
            Some(Thread {
                path: note
                    .pointer("/position/new_path")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                line: note
                    .pointer("/position/new_line")
                    .and_then(Value::as_u64)
                    .and_then(|line| u32::try_from(line).ok()),
                author: note
                    .pointer("/author/username")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                body: first_line(&text(note, "body")),
                resolved: note
                    .get("resolved")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .take(THREAD_LIMIT)
        .collect()
}
