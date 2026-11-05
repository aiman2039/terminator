//! Compare a file with a branch, a tag, a typed revision, or a commit from
//! its history (`IntelliJ` Compare with Branch or Tag and Compare with
//! Revision). The lists come from read-only Git queries on the file pool;
//! a pick opens the native revision diff against the working file.
mod picker;
#[cfg(test)]
mod tests;
mod view;

pub(crate) use picker::{ComparePicker, PickerMode};
pub(crate) use view::{REMOTE_NOTE, answers};

use std::path::Path;
use terminator_core::CommandOptions;

/// The menu rows that open the picker: label, icon, and what it lists.
pub(crate) const MENU_ITEMS: [(&str, &str, PickerMode); 2] = [
    ("Compare with…", "GitCompareArrows", PickerMode::Refs),
    ("Compare with Revision…", "History", PickerMode::History),
];

/// The kind of ref a compare target names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetKind {
    LocalBranch,
    RemoteBranch,
    Tag,
}

/// A branch or tag that a file can be compared with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompareTarget {
    pub kind: TargetKind,
    /// The short name: `main`, `origin/main`, or `v1.0`.
    pub name: String,
    /// The full ref name, such as `refs/heads/main`.
    pub full: String,
    /// The checked-out branch.
    pub current: bool,
    /// Unix time of the commit or tag; newer tags come first.
    time: i64,
    /// Another listed ref has the same short name.
    ambiguous: bool,
}

impl CompareTarget {
    /// The revision Git reads: the short name, or the full ref name when
    /// another ref has the same short name (a tag `v1` and a branch `v1`)
    /// or when the short name starts with `-`: a fetched tag such as
    /// `--output=x` must not reach Git as an option.
    #[must_use]
    pub fn revision(&self) -> &str {
        if self.ambiguous || self.name.starts_with('-') {
            &self.full
        } else {
            &self.name
        }
    }
}

/// `for-each-ref` fields: the ref name, `*` on the checked-out branch, the
/// creator date, and the target of a symbolic ref.
const REF_FORMAT: &str = "--format=%(refname)%09%(HEAD)%09%(creatordate:unix)%09%(symref)";

/// Quick read-only Git calls: a hung call fails after 20 seconds.
fn options() -> CommandOptions {
    CommandOptions {
        stdout_limit: 4 * 1024 * 1024,
        timeout: std::time::Duration::from_secs(20),
        ..Default::default()
    }
}

/// Branches, remote branches, and tags of the repository at `cwd`.
pub fn list_targets(cwd: &Path) -> Result<Vec<CompareTarget>, String> {
    let args = [
        "for-each-ref",
        REF_FORMAT,
        "refs/heads",
        "refs/remotes",
        "refs/tags",
    ];
    let bytes = terminator_core::git::run_blocking(cwd, &args, options())
        .map_err(|error| format!("git for-each-ref: {error}"))?;
    Ok(parse_targets(&String::from_utf8_lossy(&bytes)))
}

/// Local branches (the checked-out branch first, then by name), remote
/// branches by name, then tags, newest first. Symbolic refs such as
/// `origin/HEAD` are left out: they name another listed branch.
#[must_use]
pub fn parse_targets(text: &str) -> Vec<CompareTarget> {
    let mut targets: Vec<CompareTarget> = text.lines().filter_map(parse_target).collect();
    targets.sort_by(|a, b| {
        group(a.kind)
            .cmp(&group(b.kind))
            .then_with(|| b.current.cmp(&a.current))
            .then_with(|| match a.kind {
                TargetKind::Tag => b.time.cmp(&a.time),
                _ => std::cmp::Ordering::Equal,
            })
            .then_with(|| a.name.cmp(&b.name))
    });
    let mut names: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for target in &targets {
        let count = names.entry(target.name.clone()).or_default();
        *count = count.saturating_add(1);
    }
    for target in &mut targets {
        target.ambiguous = names.get(&target.name).is_some_and(|count| *count > 1);
    }
    targets
}

/// List order of each kind.
fn group(kind: TargetKind) -> u8 {
    match kind {
        TargetKind::LocalBranch => 0,
        TargetKind::RemoteBranch => 1,
        TargetKind::Tag => 2,
    }
}

/// One `for-each-ref` line, unless it is a symbolic ref.
fn parse_target(line: &str) -> Option<CompareTarget> {
    let mut fields = line.split('\t');
    let full = fields.next()?.trim();
    let head = fields.next().unwrap_or_default();
    let time = fields
        .next()
        .and_then(|time| time.trim().parse().ok())
        .unwrap_or(0);
    if !fields.next().unwrap_or_default().trim().is_empty() {
        return None;
    }
    let (kind, name) = if let Some(name) = full.strip_prefix("refs/heads/") {
        (TargetKind::LocalBranch, name)
    } else if let Some(name) = full.strip_prefix("refs/remotes/") {
        (TargetKind::RemoteBranch, name)
    } else {
        (TargetKind::Tag, full.strip_prefix("refs/tags/")?)
    };
    (!name.is_empty()).then(|| CompareTarget {
        kind,
        name: name.to_owned(),
        full: full.to_owned(),
        current: head.trim() == "*",
        time,
        ambiguous: false,
    })
}

/// `text` trimmed, when Git can only read it as a revision: not empty, no
/// leading `-` (an option), and no spaces or control characters.
pub fn typed_revision(text: &str) -> Result<&str, String> {
    let rev = text.trim();
    if rev.is_empty() {
        return Err("Type a branch, tag, or commit".into());
    }
    if rev.starts_with('-') || rev.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(format!("'{rev}' is not a revision"));
    }
    Ok(rev)
}

/// The full hash of the commit that `text` names (`git rev-parse
/// --verify`). A tree or a file (`HEAD:a.txt`) and a range are refused.
pub fn verify_revision(cwd: &Path, text: &str) -> Result<String, String> {
    let rev = typed_revision(text)?;
    let spec = format!("{rev}^{{commit}}");
    terminator_core::git::run_blocking(cwd, &["rev-parse", "--verify", "--quiet", &spec], options())
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned())
        .filter(|hash| !hash.is_empty())
        .ok_or_else(|| format!("No commit matches '{rev}'"))
}
