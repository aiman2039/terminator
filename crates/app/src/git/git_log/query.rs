//! Running a Commit Log query: the filtered `git log`, the hash and Go to
//! Hash lookups, and the graph lanes. Git runs through [`LogGit`], so a
//! background job can stop the Git of a query that a newer one replaced.
use super::filter::LOG_FORMAT;
use super::{CommitEntry, CommitLog, LogFilter, LogQuery, graph_rows, parse_commits};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use terminator_core::CommandOptions;

/// Subsystem of Commit Log query jobs.
pub const LOG_SUBSYSTEM: &str = "git-log";

/// Shown instead of a Commit Log for an SSH project.
pub const REMOTE_LOG_NOTE: &str =
    "The Commit Log needs a local repository. Use the SSH terminal for remote history.";

/// Runs one Git command in `dir` and returns its standard output.
pub trait LogGit {
    fn git(&self, dir: &Path, args: &[&str], options: CommandOptions) -> Result<Vec<u8>, String>;
}

/// Git on the calling thread.
pub struct BlockingGit;

impl LogGit for BlockingGit {
    fn git(&self, dir: &Path, args: &[&str], options: CommandOptions) -> Result<Vec<u8>, String> {
        terminator_core::git::run_blocking(dir, args, options).map_err(|error| error.to_string())
    }
}

/// Quick read-only Git calls.
fn quick() -> CommandOptions {
    CommandOptions {
        stdout_limit: 1024 * 1024,
        ..Default::default()
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_owned()
}

/// The repository root that contains `cwd`.
pub(super) fn root_with(git: &impl LogGit, cwd: &Path) -> Result<PathBuf, String> {
    let bytes = git.git(cwd, &["rev-parse", "--show-toplevel"], quick())?;
    Ok(PathBuf::from(text(&bytes)))
}

/// The log that `query` asks for, from the repository at `cwd`. A failure
/// is in `error`.
pub fn query_log(git: &impl LogGit, cwd: &Path, query: &LogQuery) -> CommitLog {
    load(git, cwd, query).unwrap_or_else(|error| query.failed(error))
}

fn load(git: &impl LogGit, cwd: &Path, query: &LogQuery) -> Result<CommitLog, String> {
    let root = root_with(git, cwd).map_err(|error| format!("Git root: {error}"))?;
    let branch = git
        .git(&root, &["rev-parse", "--abbrev-ref", "HEAD"], quick())
        .map_err(|error| format!("Branch: {error}"))?;
    let args = query.filter.log_args()?;
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let options = CommandOptions {
        stdout_limit: 4 * 1024 * 1024,
        // Ordering walks the whole history when there is no commit-graph
        // file; so do filters that match few commits.
        timeout: std::time::Duration::from_secs(20),
        ..Default::default()
    };
    let bytes = git
        .git(&root, &args, options)
        .map_err(|error| format!("git log: {error}"))?;
    let mut commits = parse_commits(&String::from_utf8_lossy(&bytes));
    if let Some(first) = hash_match(git, &root, &query.filter, &commits) {
        // The text also names this commit: it comes first.
        commits.insert(0, first);
    }
    let mut log = CommitLog {
        refs: super::fetch_refs(git, &root),
        active_branch: text(&branch),
        me: user_name(git, &root),
        query: Box::new(query.clone()),
        ..CommitLog::default()
    };
    if let Some(rev) = &query.goto {
        match find(git, &root, rev, &mut commits) {
            Ok(hash) => log.found = Some(hash),
            Err(note) => log.missing = Some(note),
        }
    }
    log.lanes = lay_out(&mut commits, query.filter.hides_commits());
    log.commits = commits;
    Ok(log)
}

/// The commit that the text names by hash, when the message search did not
/// list it.
fn hash_match(
    git: &impl LogGit,
    root: &Path,
    filter: &LogFilter,
    commits: &[CommitEntry],
) -> Option<CommitEntry> {
    let prefix = filter.hash_text()?;
    if commits
        .iter()
        .any(|commit| commit.hash.starts_with(&prefix))
    {
        return None;
    }
    let hash = resolve(git, root, &prefix).ok()?;
    record(git, root, &hash).ok()
}

/// Go to Hash: the full hash of the commit `rev` names. A commit that the
/// filters or the limit left out is added at the end.
fn find(
    git: &impl LogGit,
    root: &Path,
    rev: &str,
    commits: &mut Vec<CommitEntry>,
) -> Result<String, String> {
    let hash = resolve(git, root, rev)?;
    if !commits.iter().any(|commit| commit.hash == hash) {
        commits.push(record(git, root, &hash)?);
    }
    Ok(hash)
}

/// The full hash of the commit `rev` names (`git rev-parse --verify`).
/// Text that Git could read as an option is refused without a call.
fn resolve(git: &impl LogGit, root: &Path, rev: &str) -> Result<String, String> {
    let rev = crate::revision_compare::typed_revision(rev)?;
    let spec = format!("{rev}^{{commit}}");
    git.git(root, &["rev-parse", "--verify", "--quiet", &spec], quick())
        .ok()
        .map(|bytes| text(&bytes))
        .filter(|hash| !hash.is_empty())
        .ok_or_else(|| format!("No commit matches '{rev}'"))
}

/// One commit, read as a `git log` record.
fn record(git: &impl LogGit, root: &Path, hash: &str) -> Result<CommitEntry, String> {
    let args = [
        "log",
        "-1",
        "--decorate=full",
        "--date=unix",
        LOG_FORMAT,
        hash,
        "--",
    ];
    let bytes = git
        .git(root, &args, quick())
        .map_err(|error| format!("git log: {error}"))?;
    parse_commits(&String::from_utf8_lossy(&bytes))
        .into_iter()
        .next()
        .ok_or_else(|| format!("No commit matches '{hash}'"))
}

/// `git config user.name`, when it is set: the User filter's "me".
fn user_name(git: &impl LogGit, root: &Path) -> Option<String> {
    let bytes = git
        .git(root, &["config", "--get", "user.name"], quick())
        .ok()?;
    Some(text(&bytes)).filter(|name| !name.is_empty())
}

/// Gives each commit its graph row and returns the widest row, in lanes.
/// A line joins a commit only to a parent below it. A parent that is not
/// listed keeps its lane open to the bottom, because older history goes on
/// below the list; with `listed_only`, a filter hid that parent, so the
/// line ends at the commit.
pub(super) fn lay_out(commits: &mut [CommitEntry], listed_only: bool) -> usize {
    let rows = {
        let rows_of: HashMap<&str, usize> = commits
            .iter()
            .enumerate()
            .map(|(index, commit)| (commit.hash.as_str(), index))
            .collect();
        let parents: Vec<Vec<&str>> = commits
            .iter()
            .enumerate()
            .map(|(index, commit)| {
                commit
                    .parents
                    .iter()
                    .map(String::as_str)
                    .filter(|parent| match rows_of.get(parent) {
                        Some(row) => *row > index,
                        None => !listed_only,
                    })
                    .collect()
            })
            .collect();
        graph_rows(
            commits
                .iter()
                .zip(&parents)
                .map(|(commit, parents)| (commit.hash.as_str(), parents.as_slice())),
        )
    };
    for (commit, row) in commits.iter_mut().zip(rows) {
        commit.graph = row;
    }
    commits
        .iter()
        .map(|commit| commit.graph.width())
        .max()
        .unwrap_or(0)
}
