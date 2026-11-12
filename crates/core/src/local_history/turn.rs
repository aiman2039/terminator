//! Agent turns over Local History: which files changed since an agent's
//! last turn started, and the snapshot each file had at that start.
//!
//! A turn starts at the hook `Running` event of an agent in a terminal
//! session and ends at its next `Completed`, `WaitingInput`, `Failed` or
//! `Stopped` hook event. `WaitingPermission` does not end it: the agent
//! goes on with the same turn after the answer. Hook events only; never
//! terminal text. The daemon keeps the newest turn per session in memory.
use super::{Entry, Source, Store, read_lines};
use crate::AgentState;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Daemons that serve `LocalHistoryTurn` advertise this.
pub const TURN_CAPABILITY: &str = "local-history-turn-v1";
/// Most files in one `LocalHistoryTurn` reply.
pub const MAX_TURN_FILES: usize = 500;
/// The recorder takes a snapshot up to about a second after a change, so a
/// snapshot this soon after the turn start still shows the content from
/// before the turn. A change the agent makes this early is not reverted.
pub const START_GRACE_MS: u64 = 1000;

/// The newest turn of one session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnMark {
    pub invocation_id: String,
    pub agent_kind: String,
    pub started_ms: u64,
    /// `None` while the turn runs.
    pub ended_ms: Option<u64>,
}

/// The mark after a hook event of `invocation` in a session whose newest
/// mark is `mark`, or `None` when the event changes nothing.
#[must_use]
pub fn next_mark(
    mark: Option<&TurnMark>,
    state: AgentState,
    (invocation, kind): (&str, &str),
    now_ms: u64,
) -> Option<TurnMark> {
    let open = mark.is_some_and(|m| m.ended_ms.is_none() && m.invocation_id == invocation);
    match state {
        AgentState::Running if !open => Some(TurnMark {
            invocation_id: invocation.to_owned(),
            agent_kind: kind.to_owned(),
            started_ms: now_ms,
            ended_ms: None,
        }),
        AgentState::Completed
        | AgentState::WaitingInput
        | AgentState::Failed
        | AgentState::Stopped
            if open =>
        {
            mark.map(|m| TurnMark {
                ended_ms: Some(now_ms),
                ..m.clone()
            })
        }
        _ => None,
    }
}

/// One file Local History saw change since the turn started.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnFile {
    pub path: PathBuf,
    /// The content at the turn start, when Local History has it.
    pub base: Option<Entry>,
    /// `base` is the Git index version recorded before the file's first
    /// seen change, not a snapshot from before the turn: the content at
    /// the turn start was not recorded.
    pub base_is_git_index: bool,
    /// Id of the newest snapshot (the content Local History saw last).
    pub newest: Option<String>,
}

/// Reply of `LocalHistoryTurn`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub session: String,
    pub project_root: PathBuf,
    pub mark: TurnMark,
    pub files: Vec<TurnFile>,
    /// More files changed than [`MAX_TURN_FILES`].
    pub truncated: bool,
}

/// The turn-start snapshot of a timeline (`entries` in any order, labels
/// allowed): the newest snapshot taken by `started_ms` plus the grace, else
/// the Git index baseline recorded before the first change.
#[must_use]
pub fn turn_base(entries: &[Entry], started_ms: u64) -> (Option<Entry>, bool) {
    let mut snapshots: Vec<&Entry> = entries
        .iter()
        .filter(|e| e.source != Source::Label && !e.id.is_empty())
        .collect();
    snapshots.sort_by_key(|e| e.timestamp_ms);
    let limit = started_ms.saturating_add(START_GRACE_MS);
    if let Some(before) = snapshots.iter().rev().find(|e| e.timestamp_ms <= limit) {
        return (Some((*before).clone()), false);
    }
    match snapshots.first() {
        Some(first) if first.source == Source::Open => (Some((*first).clone()), true),
        _ => (None, false),
    }
}

impl Store {
    /// Files under `root` with a snapshot newer than `started_ms`, each
    /// with its turn-start snapshot, sorted by path; at most `limit`, and
    /// whether more exist. Index files untouched since the start are not
    /// read.
    pub fn turn_files(
        &self,
        root: &Path,
        started_ms: u64,
        limit: usize,
    ) -> super::Result<(Vec<TurnFile>, bool)> {
        let mut files = Vec::new();
        let mut truncated = false;
        // A coarse file clock must not hide an index written at the start.
        let since = std::time::UNIX_EPOCH
            .checked_add(std::time::Duration::from_millis(
                started_ms.saturating_sub(2000),
            ))
            .unwrap_or(std::time::UNIX_EPOCH);
        for index in super::list_dir(&self.root.join("files"))? {
            let fresh = std::fs::metadata(&index)
                .and_then(|meta| meta.modified())
                .is_ok_and(|modified| modified >= since);
            if !fresh {
                continue;
            }
            let entries: Vec<Entry> = read_lines(&index)?;
            let Some(path) = entries.first().map(|e| e.path.clone()) else {
                continue;
            };
            if !path.starts_with(root) || !entries.iter().any(|e| e.timestamp_ms > started_ms) {
                continue;
            }
            if files.len() >= limit {
                truncated = true;
                break;
            }
            let (base, base_is_git_index) = turn_base(&entries, started_ms);
            let newest = entries
                .iter()
                .max_by_key(|e| e.timestamp_ms)
                .map(|e| e.id.clone());
            files.push(TurnFile {
                path,
                base,
                base_is_git_index,
                newest,
            });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok((files, truncated))
    }
}

#[cfg(test)]
mod tests {
    use super::super::Record;
    use super::*;

    fn record(store: &Store, path: &Path, bytes: &[u8], (source, now_ms): (Source, u64)) {
        store
            .record(Record {
                path,
                bytes,
                source,
                label: None,
                now_ms,
            })
            .unwrap();
    }

    #[test]
    fn a_turn_starts_on_running_and_ends_on_completed_or_waiting_input() {
        let ids = ("inv", "claude");
        let start = next_mark(None, AgentState::Running, ids, 10).unwrap();
        assert_eq!(start.started_ms, 10);
        assert!(next_mark(Some(&start), AgentState::Running, ids, 20).is_none());
        assert!(
            next_mark(Some(&start), AgentState::WaitingPermission, ids, 30).is_none(),
            "a permission prompt keeps the turn open"
        );
        let end = next_mark(Some(&start), AgentState::Completed, ids, 40).unwrap();
        assert_eq!((end.started_ms, end.ended_ms), (10, Some(40)));
        assert!(next_mark(Some(&end), AgentState::WaitingInput, ids, 50).is_none());
        let again = next_mark(Some(&end), AgentState::Running, ids, 60).unwrap();
        assert_eq!((again.started_ms, again.ended_ms), (60, None));
        let other = next_mark(Some(&start), AgentState::Running, ("new", "codex"), 70).unwrap();
        assert_eq!(other.invocation_id, "new");
    }

    #[test]
    fn turn_files_lists_only_files_changed_after_the_start_with_their_start_content() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_data(dir.path());
        let root = Path::new("/p");
        let (old, fresh, new, other) = (
            Path::new("/p/old.rs"),
            Path::new("/p/fresh.rs"),
            Path::new("/p/new.rs"),
            Path::new("/q/x.rs"),
        );
        let (early, late) = ((Source::External, 1_000), (Source::External, 20_000));
        record(&store, old, b"before", early);
        record(&store, fresh, b"before", early);
        record(&store, fresh, b"during", late);
        record(&store, new, b"created", late);
        record(&store, other, b"x", late);
        store
            .label(root, super::super::BEFORE_AGENT_RUN, 10_000)
            .unwrap();
        let (files, truncated) = store.turn_files(root, 10_000, 10).unwrap();
        assert!(!truncated);
        let paths: Vec<&Path> = files.iter().map(|f| f.path.as_path()).collect();
        assert_eq!(paths, [fresh, new]);
        let base = files[0].base.as_ref().unwrap();
        assert_eq!(store.read(&base.id).unwrap(), "before");
        assert_eq!(
            files[1].base, None,
            "a file created in the turn has no base"
        );
        let (files, truncated) = store.turn_files(root, 10_000, 1).unwrap();
        assert!(truncated && files.len() == 1);
    }

    #[test]
    fn git_index_baseline_is_a_flagged_base_and_early_snapshots_count_as_start() {
        let at = |id: &str, ts, source| Entry {
            id: id.repeat(64),
            path: PathBuf::from("/p/a"),
            timestamp_ms: ts,
            size: 1,
            label: None,
            source,
        };
        let index = [
            at("a", 19_999, Source::Open),
            at("b", 20_000, Source::External),
        ];
        let (base, flagged) = turn_base(&index, 10_000);
        assert_eq!(base.unwrap().id, "a".repeat(64));
        assert!(flagged);
        let early = [
            at("a", 1, Source::External),
            at("b", 10_500, Source::External),
        ];
        let (base, flagged) = turn_base(&early, 10_000);
        assert_eq!(base.unwrap().id, "b".repeat(64), "inside the grace");
        assert!(!flagged);
    }
}
