//! The Compare popup's state: what it lists, the search, the selection,
//! and what a pick asks for. No painting and no I/O here.
use super::{CompareTarget, TargetKind};
use crate::git_log::HistoryEntry;
use crate::workspace_ops::{Op, Report};
use std::path::{Path, PathBuf};

/// What the picker lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerMode {
    /// Branches, remote branches, tags, and a typed revision.
    Refs,
    /// The commits that changed the file.
    History,
}

impl From<terminator_native_edit::view::GitCompare> for PickerMode {
    fn from(pick: terminator_native_edit::view::GitCompare) -> Self {
        match pick {
            terminator_native_edit::view::GitCompare::Refs => Self::Refs,
            terminator_native_edit::view::GitCompare::History => Self::History,
        }
    }
}

/// The loaded choices.
#[derive(Debug)]
pub(crate) enum Choices {
    Loading,
    Failed(String),
    Targets(Vec<CompareTarget>),
    Commits(Vec<HistoryEntry>),
}

/// One row of the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Row<'a> {
    Header(&'static str),
    Target(&'a CompareTarget),
    Commit(&'a HistoryEntry),
    /// The search text, to check as a revision.
    Typed(&'a str),
}

/// What picking a row asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Pick {
    /// Compare with this revision now.
    Open(String),
    /// Check first that this typed text names a commit.
    Verify(String),
}

impl Row<'_> {
    /// The pick this row makes; headers make none.
    pub(crate) fn pick(&self) -> Option<Pick> {
        match self {
            Row::Header(_) => None,
            Row::Target(target) => Some(Pick::Open(target.revision().to_owned())),
            Row::Commit(entry) => Some(Pick::Open(entry.hash.clone())),
            Row::Typed(text) => Some(Pick::Verify((*text).to_owned())),
        }
    }
}

/// The Compare popup for one file.
pub(crate) struct ComparePicker {
    pub(crate) mode: PickerMode,
    /// Where Git runs: the file's folder, so a file in a nested
    /// repository compares against its own repository.
    pub(crate) cwd: PathBuf,
    pub(crate) path: PathBuf,
    pub(crate) query: String,
    pub(crate) choices: Choices,
    /// Position among the rows that a pick can choose.
    pub(crate) selected: usize,
    /// The typed revision that Git is checking.
    pub(crate) checking: Option<String>,
    /// Why the last typed revision was refused.
    pub(crate) note: Option<String>,
    /// Focus the search field on the next paint.
    pub(crate) focus: bool,
}

impl ComparePicker {
    /// A picker for `path`, still loading; `None` for a path with no folder.
    pub(crate) fn new(mode: PickerMode, path: &Path) -> Option<Self> {
        let cwd = path.parent()?.to_path_buf();
        Some(Self {
            mode,
            cwd,
            path: path.to_owned(),
            query: String::new(),
            choices: Choices::Loading,
            selected: 0,
            checking: None,
            note: None,
            focus: true,
        })
    }

    /// The window title.
    pub(crate) fn title(&self) -> &'static str {
        match self.mode {
            PickerMode::Refs => "Compare with…",
            PickerMode::History => "Compare with Revision",
        }
    }

    /// The query that loads this picker's choices.
    pub(crate) fn load_op(&self) -> Op {
        match self.mode {
            PickerMode::Refs => Op::CompareTargets,
            PickerMode::History => Op::FileHistory(self.path.clone()),
        }
    }

    /// `op` is the query that loads this picker.
    fn loads(&self, op: &Op) -> bool {
        match (self.mode, op) {
            (PickerMode::Refs, Op::CompareTargets) => true,
            (PickerMode::History, Op::FileHistory(path)) => *path == self.path,
            _ => false,
        }
    }

    /// The diff of `revision` against the working file.
    pub(crate) fn diff_tab(&self, revision: String) -> crate::Tab {
        crate::Tab::RevisionDiff {
            cwd: self.cwd.clone(),
            path: self.path.clone(),
            left: revision,
            right: String::new(),
        }
    }

    /// Takes a Git answer meant for this picker; answers for another
    /// mode, file, or typed text are stale and change nothing. Returns the
    /// diff to open once a typed revision checks out: against the commit
    /// it named then, by full hash.
    pub(crate) fn answer(&mut self, op: &Op, result: Result<Report, String>) -> Option<crate::Tab> {
        if let Op::VerifyRevision(text) = op {
            if self.checking.as_deref() != Some(text.as_str()) {
                return None;
            }
            self.checking = None;
            return match result {
                Ok(Report::Revision(hash)) => Some(self.diff_tab(hash)),
                Ok(_) => None,
                Err(error) => {
                    self.note = Some(error);
                    None
                }
            };
        }
        if !self.loads(op) {
            return None;
        }
        self.choices = match result {
            Ok(Report::CompareTargets(targets)) => Choices::Targets(targets),
            Ok(Report::History(commits)) => Choices::Commits(commits),
            Ok(_) => return None,
            Err(error) => Choices::Failed(error),
        };
        None
    }
}

/// The rows for `query`: each kind of ref under its header, then the
/// query itself as a revision unless it names a listed ref; or the
/// commits whose subject, author, or hash matches.
pub(crate) fn rows<'a>(choices: &'a Choices, query: &'a str) -> Vec<Row<'a>> {
    let needle = query.trim();
    match choices {
        Choices::Targets(targets) => target_rows(targets, needle),
        Choices::Commits(commits) => commits
            .iter()
            .filter(|entry| commit_matches(entry, needle))
            .map(Row::Commit)
            .collect(),
        Choices::Loading | Choices::Failed(_) => Vec::new(),
    }
}

fn target_rows<'a>(targets: &'a [CompareTarget], needle: &'a str) -> Vec<Row<'a>> {
    let mut rows = Vec::new();
    for (kind, header) in [
        (TargetKind::LocalBranch, "LOCAL BRANCHES"),
        (TargetKind::RemoteBranch, "REMOTE BRANCHES"),
        (TargetKind::Tag, "TAGS"),
    ] {
        let mut group = targets
            .iter()
            .filter(|target| target.kind == kind && contains(&target.name, needle))
            .peekable();
        if group.peek().is_some() {
            rows.push(Row::Header(header));
            rows.extend(group.map(Row::Target));
        }
    }
    let named = targets
        .iter()
        .any(|target| target.name == needle || target.full == needle);
    if !needle.is_empty() && !named {
        rows.push(Row::Header("REVISION"));
        rows.push(Row::Typed(needle));
    }
    rows
}

fn commit_matches(entry: &HistoryEntry, needle: &str) -> bool {
    contains(&entry.subject, needle)
        || contains(&entry.author, needle)
        || entry
            .hash
            .get(..needle.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(needle))
}

/// `needle` is in `text`, ignoring ASCII case; an empty needle always is.
fn contains(text: &str, needle: &str) -> bool {
    needle.is_empty()
        || text
            .as_bytes()
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

/// The selection one pickable row down or up from `selected`, wrapping
/// within `count` rows.
pub(crate) fn stepped(selected: usize, down: bool, count: usize) -> usize {
    let last = count.saturating_sub(1);
    if count == 0 {
        0
    } else if down {
        selected.saturating_add(1).checked_rem(count).unwrap_or(0)
    } else {
        selected.min(last).checked_sub(1).unwrap_or(last)
    }
}

/// Indices of the rows that a pick can choose (every row but headers).
pub(crate) fn pickable(rows: &[Row<'_>]) -> Vec<usize> {
    rows.iter()
        .enumerate()
        .filter(|(_, row)| !matches!(row, Row::Header(_)))
        .map(|(index, _)| index)
        .collect()
}
