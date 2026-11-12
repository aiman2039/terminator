//! Read-only issue and pull request lookup for New task: `gh` or `glab`,
//! picked by the `origin` host as Create PR does. Blocking: it runs on the
//! file pool with the 20-second cap and never prompts.
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use terminator_core::{CommandOptions, git, run_command};

/// Cap of one CLI call, as for the other quick Git panel calls.
const CALL_CAP: Duration = Duration::from_secs(20);
/// Cap of the pull request head fetch on Create, as for Git panel Fetch.
const FETCH_CAP: Duration = Duration::from_mins(5);

/// What the user typed: a number, `#12`, `!12`, or an issue, pull request
/// or merge request URL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Reference {
    pub number: u64,
    /// `Some(true)` for a pull or merge request, `Some(false)` for an
    /// issue, `None` when the input does not say.
    pub pull: Option<bool>,
}

/// A fetched issue or pull request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Issue {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub url: String,
    /// `Some` for a pull or merge request: the ref its head is fetched from.
    pub head_ref: Option<String>,
}

pub(crate) fn parse_reference(input: &str) -> Option<Reference> {
    let input = input.trim();
    let digits = |text: &str| {
        (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
            .then(|| text.parse::<u64>().ok())
            .flatten()
            .filter(|n| *n > 0)
    };
    if let Some(rest) = input.strip_prefix('#') {
        return digits(rest).map(|number| Reference { number, pull: None });
    }
    if let Some(rest) = input.strip_prefix('!') {
        return digits(rest).map(|number| Reference {
            number,
            pull: Some(true),
        });
    }
    if let Some(number) = digits(input) {
        return Some(Reference { number, pull: None });
    }
    let url = url::Url::parse(input).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let segments: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    let [.., kind, number] = segments.as_slice() else {
        return None;
    };
    let pull = match *kind {
        "issues" => false,
        "pull" | "pulls" | "merge_requests" => true,
        _ => return None,
    };
    digits(number).map(|number| Reference {
        number,
        pull: Some(pull),
    })
}

fn call(program: &Path, root: &Path, args: &[String]) -> Result<Value, String> {
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
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        return Err(error.lines().next().unwrap_or("Failed").trim().to_owned());
    }
    serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// `gh issue view` / `gh pr view` JSON.
pub(crate) fn parse_gh(value: &Value, pull: bool) -> Issue {
    let number = value
        .get("number")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    Issue {
        number,
        title: text(value, "title"),
        body: text(value, "body"),
        url: text(value, "url"),
        head_ref: pull.then(|| format!("refs/pull/{number}/head")),
    }
}

/// `glab issue view` / `glab mr view` JSON.
pub(crate) fn parse_glab(value: &Value, pull: bool) -> Issue {
    let number = value.get("iid").and_then(Value::as_u64).unwrap_or_default();
    Issue {
        number,
        title: text(value, "title"),
        body: text(value, "description"),
        url: text(value, "web_url"),
        head_ref: pull.then(|| format!("refs/merge-requests/{number}/head")),
    }
}

/// The view arguments for one kind. The number is digits only, so it can
/// never become an option.
fn view_args(cli: &str, pull: bool, number: u64) -> Vec<String> {
    let words: &[&str] = match (cli, pull) {
        ("glab", false) => &["issue", "view"],
        ("glab", true) => &["mr", "view"],
        (_, false) => &["issue", "view"],
        (_, true) => &["pr", "view"],
    };
    let mut args: Vec<String> = words.iter().map(|w| (*w).to_owned()).collect();
    args.push(number.to_string());
    if cli == "glab" {
        args.extend(["--output".to_owned(), "json".to_owned()]);
    } else {
        let fields = if pull {
            "number,title,body,url,headRefName"
        } else {
            "number,title,body,url"
        };
        args.extend(["--json".to_owned(), fields.to_owned()]);
    }
    args
}

/// Looks the reference up in the `origin` repository of `root`. A bare
/// number is tried as an issue first, then as a pull request. `program`
/// replaces the CLI found on `PATH` (tests use a fake one).
pub(crate) fn fetch(root: &Path, input: &str, program: Option<&Path>) -> Result<Issue, String> {
    let reference =
        parse_reference(input).ok_or("Type an issue or pull request number, or its URL.")?;
    let options = CommandOptions {
        timeout: CALL_CAP,
        ..Default::default()
    };
    let remote = run_command(
        git::command(root, &["remote", "get-url", "origin"]),
        options,
    )
    .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    .map_err(|_| "Fetching an issue needs an origin remote.".to_owned())?;
    let cli = crate::workspace_ops::review_cli(&remote);
    let program = program
        .map(PathBuf::from)
        .or_else(|| terminator_core::find_executable(cli))
        .filter(|path| path.is_file())
        .ok_or_else(|| format!("Install {cli} to fetch issues and pull requests."))?;
    let kinds: &[bool] = match reference.pull {
        Some(pull) => {
            if pull {
                &[true]
            } else {
                &[false]
            }
        }
        None => &[false, true],
    };
    let mut last = String::new();
    for &pull in kinds {
        match call(&program, root, &view_args(cli, pull, reference.number)) {
            Ok(value) if cli == "glab" => return Ok(parse_glab(&value, pull)),
            Ok(value) => return Ok(parse_gh(&value, pull)),
            Err(error) => last = error,
        }
    }
    Err(format!("{cli}: {last}"))
}

/// On Create: fetches the pull request head from `origin` into
/// `FETCH_HEAD` (no branch or ref changes) and returns its commit.
pub(crate) fn fetch_head(root: &Path, head_ref: &str) -> anyhow::Result<String> {
    anyhow::ensure!(
        head_ref.starts_with("refs/") && !head_ref.contains(char::is_whitespace),
        "Unexpected pull request ref"
    );
    run_command(
        git::command(root, &["fetch", "--no-tags", "origin", head_ref]),
        CommandOptions {
            timeout: FETCH_CAP,
            ..Default::default()
        },
    )?;
    let output = run_command(
        git::command(root, &["rev-parse", "--verify", "FETCH_HEAD^{commit}"]),
        CommandOptions {
            timeout: CALL_CAP,
            ..Default::default()
        },
    )?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
