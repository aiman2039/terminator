//! Commit Log filters (the `IntelliJ` Log filter bar) and the `git log`
//! arguments they map to. A typed value never reaches Git as an option:
//! patterns go in `--grep=` and `--author=` forms, a branch as its full ref
//! name, and the path after `--`.
use super::CommitLog;

/// `git log` record: hash, short hash, author, email, date, parents, refs,
/// then the message.
pub(super) const LOG_FORMAT: &str = "--format=%H%n%h%n%an%n%ae%n%ad%n%P%n%D%n%B%n>>>>>END<<<<<";

/// The newest commits one load lists.
const LOG_LIMIT: &str = "--max-count=1000";

/// Commits the Date filter keeps, by commit date.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LogDate {
    #[default]
    Any,
    Day,
    Week,
    Month,
}

impl LogDate {
    pub const ALL: [Self; 4] = [Self::Any, Self::Day, Self::Week, Self::Month];

    /// The menu label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Any => "Any time",
            Self::Day => "Last 24 hours",
            Self::Week => "Last 7 days",
            Self::Month => "Last 30 days",
        }
    }

    /// The `--since` value: a Git approximate date.
    fn since(self) -> Option<&'static str> {
        match self {
            Self::Any => None,
            Self::Day => Some("24.hours.ago"),
            Self::Week => Some("7.days.ago"),
            Self::Month => Some("30.days.ago"),
        }
    }
}

/// The refs the log walks.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum LogBranch {
    /// Every branch, remote branch, and tag (not the stash).
    #[default]
    All,
    /// The checked-out commit and its history.
    Head,
    /// One local or remote branch by full ref name, such as `refs/heads/main`.
    Ref(String),
}

/// The Commit Log filter bar. The default shows the whole log.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct LogFilter {
    /// Text in the commit message (any case), or a commit hash.
    pub text: String,
    pub branch: LogBranch,
    /// An author name; `None` shows every user.
    pub author: Option<String>,
    pub date: LogDate,
    /// A file or folder, relative to the repository root: only commits that
    /// change it.
    pub path: String,
}

impl LogFilter {
    /// No filter is set: the whole log.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text().is_empty()
            && self.branch == LogBranch::All
            && self.author.is_none()
            && self.date == LogDate::Any
            && self.path.trim().is_empty()
    }

    /// The Text and User filters leave out commits between the listed ones.
    /// Graph lines then join only listed commits.
    #[must_use]
    pub fn hides_commits(&self) -> bool {
        !self.text().is_empty() || self.author.is_some()
    }

    /// The text when it can be a commit hash: 7 to 64 hex digits, in lower
    /// case.
    #[must_use]
    pub fn hash_text(&self) -> Option<String> {
        let text = self.text();
        ((7..=64).contains(&text.len()) && text.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .then(|| text.to_ascii_lowercase())
    }

    /// The trimmed text without control characters.
    fn text(&self) -> String {
        clean(&self.text)
    }

    /// `git log` arguments for the newest 1000 matching commits, children
    /// before parents. Text and names match as plain text in any case.
    pub fn log_args(&self) -> Result<Vec<String>, String> {
        let mut args = vec!["log".to_owned()];
        match &self.branch {
            // Every ref but the stash: stash entries are not history.
            LogBranch::All => args.extend(["--exclude=refs/stash".into(), "--all".into()]),
            LogBranch::Head => args.push("HEAD".into()),
            LogBranch::Ref(name) => args.push(full_ref(name)?.to_owned()),
        }
        // Newest first, but never a parent before its children: the graph
        // needs a topological order. Full ref names tell a local `a/b` from
        // a remote `origin/b`.
        for arg in ["--date-order", "--decorate=full", "--date=unix"] {
            args.push(arg.into());
        }
        args.extend([LOG_FORMAT.into(), LOG_LIMIT.into()]);
        let text = self.text();
        let author = self.author.as_deref().map(clean);
        if !text.is_empty() || author.is_some() {
            args.extend(["--regexp-ignore-case".into(), "--extended-regexp".into()]);
        }
        if !text.is_empty() {
            args.push(format!("--grep={}", literal(&text)));
        }
        if let Some(author) = author {
            // The User menu lists names as committed (`%an`). Git matches
            // `--author` after `.mailmap` by default, so turn that off.
            args.push("--no-use-mailmap".into());
            // `Name <` ends at the name: `Bob` matches neither `Bobby` nor
            // `Jim Bob`.
            args.push(format!("--author=^{} <", literal(&author)));
        }
        if let Some(since) = self.date.since() {
            args.push(format!("--since={since}"));
        }
        let path = clean(&self.path);
        if !path.is_empty() {
            // Each commit's parents become the nearest older commits that
            // change the path, so graph lines stay joined.
            args.push("--parents".into());
        }
        // Revisions end here: a ref is never read as a path.
        args.push("--".into());
        if !path.is_empty() {
            args.push(path);
        }
        Ok(args)
    }
}

/// `text` trimmed, without control characters.
fn clean(text: &str) -> String {
    text.trim().chars().filter(|c| !c.is_control()).collect()
}

/// `name` when it is a full branch or remote branch ref name.
fn full_ref(name: &str) -> Result<&str, String> {
    let full = (name.starts_with("refs/heads/") || name.starts_with("refs/remotes/"))
        && !name.chars().any(|c| c.is_whitespace() || c.is_control());
    full.then_some(name)
        .ok_or_else(|| format!("'{name}' is not a branch"))
}

/// `text` as an extended regular expression that matches it literally.
fn literal(text: &str) -> String {
    let mut pattern = String::with_capacity(text.len());
    for c in text.chars() {
        if "\\.[](){}*+?^$|".contains(c) {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern
}

/// One Commit Log load: the filters, a Go to Hash target, and the number
/// that ties the reply to its request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogQuery {
    pub filter: LogFilter,
    /// A hash, branch, or tag to list and select, also when the filters or
    /// the 1000-commit limit leave it out.
    pub goto: Option<String>,
    pub generation: u64,
}

impl LogQuery {
    /// The reply when the load fails; it still names the query it answers.
    #[must_use]
    pub fn failed(&self, error: String) -> CommitLog {
        CommitLog {
            error: Some(error),
            query: Box::new(self.clone()),
            ..CommitLog::default()
        }
    }
}

impl CommitLog {
    /// The loaded commit `rev` names: a hash prefix of 4 or more hex digits,
    /// or a branch or tag label on the commit.
    #[must_use]
    pub fn find_loaded(&self, rev: &str) -> Option<&super::CommitEntry> {
        let rev = rev.trim();
        let prefix = (rev.len() >= 4 && rev.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .then(|| rev.to_ascii_lowercase());
        let by_hash = prefix.and_then(|prefix| {
            self.commits
                .iter()
                .find(|commit| commit.hash.starts_with(&prefix))
        });
        by_hash.or_else(|| {
            self.commits
                .iter()
                .find(|commit| commit.refs.iter().any(|label| label.name == rev))
        })
    }
}
