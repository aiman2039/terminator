//! Git actions run from the sidebar: stage, shelve, branches, compare.
//! Commit and push are long jobs; see `commit.rs`.
use super::*;

/// Cap for quick Git calls (staging, shelves, branches, log): a hung call
/// fails fast. Commit and push have their own long cap.
pub(super) const QUICK_GIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

pub(super) fn git_options() -> CommandOptions {
    CommandOptions {
        timeout: QUICK_GIT_TIMEOUT,
        ..Default::default()
    }
}

pub(super) fn git_text(root: &Path, args: &[&str]) -> Result<String> {
    let output = run_command(git::command(root, args), git_options())?;
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

pub(super) struct GitPathInput<'a> {
    pub(super) root: &'a Path,
    pub(super) prefix: &'a [&'a str],
    pub(super) path: &'a Path,
    pub(super) verb: &'a str,
}

pub(super) fn git_path(input: GitPathInput<'_>) -> Result<Report> {
    let GitPathInput {
        root,
        prefix,
        path,
        verb,
    } = input;
    let relative = relative_arg(root, path)?;
    let pathspec = literal(&relative);
    let mut args: Vec<&str> = prefix.to_vec();
    args.push(&pathspec);
    git_text(root, &args)?;
    Ok(Report::Message(format!("{verb} {relative}")))
}

/// A pathspec that matches only this exact path: no glob or magic.
pub(super) fn literal(relative: &str) -> String {
    format!(":(literal){relative}")
}

/// Stage many paths in one `git add`; pathspecs go through stdin so a large
/// change set does not hit the argument limit.
pub(super) fn stage_paths(root: &Path, paths: &[PathBuf]) -> Result<Report> {
    if paths.is_empty() {
        return Ok(Report::Message("Nothing to stage".into()));
    }
    let mut input = Vec::new();
    for path in paths {
        input.extend_from_slice(literal(&relative_arg(root, path)?).as_bytes());
        input.push(0);
    }
    let mut options = git_options();
    options.input = Some(input);
    let args = ["add", "--pathspec-from-file=-", "--pathspec-file-nul"];
    run_command(git::command(root, &args), options)?;
    let count = paths.len();
    let plural = if count == 1 { "" } else { "s" };
    Ok(Report::Message(format!("Staged {count} file{plural}")))
}

/// Only `stash@{N}` references may reach `git stash pop/drop`.
pub(super) fn valid_shelf_ref(reference: &str) -> Result<()> {
    let inner = reference
        .strip_prefix("stash@{")
        .and_then(|rest| rest.strip_suffix('}'))
        .unwrap_or_default();
    ensure!(
        !inner.is_empty() && inner.chars().all(|c| c.is_ascii_digit()),
        "Invalid shelf reference"
    );
    Ok(())
}

pub(super) fn shelve(root: &Path, paths: &[PathBuf]) -> Result<Report> {
    ensure!(!paths.is_empty(), "Nothing staged to shelve");
    let mut relatives = Vec::new();
    for path in paths {
        relatives.push(literal(&relative_arg(root, path)?));
    }
    let mut args: Vec<&str> = vec![
        "stash",
        "push",
        "--staged",
        "-m",
        "Shelve (Terminator)",
        "--",
    ];
    args.extend(relatives.iter().map(String::as_str));
    git_text(root, &args)?;
    Ok(Report::Message("Shelved staged changes".into()))
}

/// Shelves as `(stash@{N}, message, commit hash)`, newest first.
fn stash_list(root: &Path) -> Result<Vec<(String, String, String)>> {
    let text = git_text(root, &["stash", "list", "--format=%gd%x09%H%x09%gs"])?;
    Ok(text
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            let (reference, hash) = (fields.next()?, fields.next()?);
            let message = fields.next().unwrap_or_default();
            Some((reference.to_owned(), message.to_owned(), hash.to_owned()))
        })
        .collect())
}

/// Each shelf as `(commit hash, message)`. Rows keep the hash, not the
/// position: a `git stash` elsewhere moves every `stash@{N}`.
pub(super) fn shelves(root: &Path) -> Result<Vec<(String, String)>> {
    Ok(stash_list(root)?
        .into_iter()
        .map(|(_, message, hash)| (hash, message))
        .collect())
}

/// The current `stash@{N}` of the shelf with commit `hash`, resolved just
/// before `pop`/`drop`; a shelf that is gone is refused.
pub(super) fn shelf_position(root: &Path, hash: &str) -> Result<String> {
    ensure!(
        !hash.is_empty() && hash.chars().all(|c| c.is_ascii_hexdigit()),
        "Invalid shelf reference"
    );
    let reference = stash_list(root)?
        .into_iter()
        .find(|(_, _, listed)| listed == hash)
        .map(|(reference, _, _)| reference)
        .context("The shelf is gone; refresh the list")?;
    valid_shelf_ref(&reference)?;
    Ok(reference)
}

pub(super) fn branches(root: &Path) -> Result<Vec<String>> {
    let text = git_text(root, &["branch", "--format=%(refname:short)"])?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

pub(super) fn switch(root: &Path, name: &str) -> Result<Report> {
    ensure!(
        name.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/' | '.'))
            && !name.starts_with('-')
            && !name.split('/').any(|part| part == ".." || part.is_empty()),
        "Branch name is not allowed"
    );
    git_text(root, &["switch", "--", name])?;
    Ok(Report::Message(format!("Switched to {name}")))
}

pub(super) fn log(root: &Path) -> Result<Vec<(String, String)>> {
    let text = git_text(root, &["log", "-n", "40", "--pretty=format:%h%x09%s"])?;
    Ok(text
        .lines()
        .filter_map(|line| {
            let (hash, subject) = line.split_once('\t')?;
            Some((hash.to_owned(), subject.to_owned()))
        })
        .collect())
}

pub(super) fn git_bytes(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    Ok(run_command(git::command(root, args), git_options())?.stdout)
}

/// A base ref must not look like a flag and must resolve to a commit.
pub(super) fn valid_ref(root: &Path, base: &str) -> bool {
    !base.is_empty()
        && !base.starts_with('-')
        && !base.contains("..")
        && !base.chars().any(char::is_whitespace)
        && base
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/' | '.' | '@'))
        && git_text(
            root,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{base}^{{commit}}"),
            ],
        )
        .is_ok_and(|out| !out.is_empty())
}

/// Parse `git diff --name-status -z`. Rename/copy records carry `old\0new`; the
/// destination path wins.
pub(super) fn parse_name_status(root: &Path, raw: &[u8]) -> Vec<(PathBuf, char)> {
    let mut result = Vec::new();
    let mut parts = raw.split(|b| *b == 0).filter(|part| !part.is_empty());
    while let Some(status) = parts.next() {
        let letter = *status.first().unwrap_or(&b' ') as char;
        let Some(first) = parts.next() else {
            break;
        };
        let path = if matches!(letter, 'R' | 'C') {
            match parts.next() {
                Some(new) => new,
                None => first,
            }
        } else {
            first
        };
        result.push((root.join(os_name(path)), letter));
    }
    result
}

pub(super) fn compare(root: &Path, base_override: Option<&str>) -> Result<CompareData> {
    let mut data = CompareData {
        root: Some(root.to_path_buf()),
        ..Default::default()
    };
    if let Ok(upstream) = git_text(
        root,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    ) && !upstream.is_empty()
    {
        data.upstream = Some(upstream);
    }
    if let Some(upstream) = &data.upstream
        && let Ok(counts) = git_text(
            root,
            &[
                "rev-list",
                "--left-right",
                "--count",
                &format!("{upstream}...HEAD"),
            ],
        )
    {
        let mut parts = counts.split_whitespace();
        data.behind = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        data.ahead = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    }
    let base = base_override
        .map(str::to_owned)
        .or_else(|| data.upstream.clone());
    let Some(base) = base.filter(|base| valid_ref(root, base)) else {
        return Ok(data);
    };
    let range = format!("{base}...HEAD");
    let name_status = git_bytes(root, &["diff", "--name-status", "-z", &range]).unwrap_or_default();
    let numstat = git_bytes(root, &["diff", "--numstat", "-z", &range]).unwrap_or_default();
    let mut stats = terminator_git::parse_numstat(root, &numstat);
    data.base = Some(base);
    for (path, letter) in parse_name_status(root, &name_status) {
        let (added, deleted) = stats.remove(&path).unwrap_or((0, 0));
        data.files.push(CommittedFile {
            path,
            letter,
            added,
            deleted,
        });
    }
    Ok(data)
}

pub(super) fn unshelve(root: &Path, hash: &str) -> Result<Report> {
    let reference = shelf_position(root, hash)?;
    git_text(root, &["stash", "pop", "--quiet", &reference])?;
    Ok(Report::Message("Unshelved".into()))
}

pub(super) fn drop_shelf(root: &Path, hash: &str) -> Result<Report> {
    let reference = shelf_position(root, hash)?;
    git_text(root, &["stash", "drop", "--quiet", &reference])?;
    Ok(Report::Message("Shelf dropped".into()))
}

pub(super) fn head_message(root: &Path) -> Result<Report> {
    Ok(Report::HeadMessage(git_text(
        root,
        &["log", "-1", "--format=%B"],
    )?))
}
