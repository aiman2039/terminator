//! One Commit Log tab's filter bar state: the filters as edited, the pause
//! in typing before they reach Git, the menu choices learned from loads,
//! and Go to Hash. No painting and no I/O here.
use super::{CommitLog, LogFilter, LogQuery, RefKind};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Typing in the text or path field waits this long before the log loads.
pub const TYPING_PAUSE: Duration = Duration::from_millis(400);

/// A new number for each query, across all Commit Log tabs.
fn next_generation() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// What to do with a load's reply.
#[derive(Debug, PartialEq, Eq)]
pub enum Answer {
    /// A newer query was sent: drop the reply.
    Stale,
    /// Show the reply; select this commit (Go to Hash), if any.
    Show(Option<String>),
    /// The filtered load failed: keep the shown log; the bar shows why.
    KeepShown,
}

#[derive(Debug, Default)]
pub struct LogView {
    /// The filters as the bar shows them.
    pub draft: LogFilter,
    /// The last keystroke in the text or path field that waits to load.
    typed: Option<Instant>,
    /// The filters of the last query sent.
    sent: Option<LogFilter>,
    /// The number of the last query sent.
    generation: u64,
    /// The last query sent has no reply yet.
    pub loading: bool,
    /// Why the last filtered load failed; the older log stays shown.
    pub error: Option<String>,
    /// The Go to Hash field.
    pub goto: String,
    /// Why the last Go to Hash found nothing.
    pub goto_note: Option<String>,
    /// The revision that a Go to Hash load looks for.
    pub finding: Option<String>,
    /// Scroll the list to this commit on the next paint.
    pub reveal: Option<String>,
    /// Authors seen in this tab's loads, for the User menu.
    pub authors: BTreeSet<String>,
    /// Local, then remote branches: `(full ref name, short name)`.
    pub branches: Vec<(String, String)>,
    /// `git config user.name`: the User menu's "me".
    pub me: Option<String>,
}

impl LogView {
    /// A keystroke in the text or path field: load after a pause.
    pub fn typed(&mut self, now: Instant) {
        self.typed = Some(now);
    }

    /// A menu pick or Enter: load at once.
    pub fn apply(&mut self) {
        self.typed = None;
    }

    /// Clear filters: the whole log, at once.
    pub fn clear(&mut self) {
        self.draft = LogFilter::default();
        self.apply();
    }

    /// Time left before typed changes load.
    #[must_use]
    pub fn wait(&self, now: Instant) -> Option<Duration> {
        let typed = self.typed?;
        TYPING_PAUSE
            .checked_sub(now.saturating_duration_since(typed))
            .filter(|left| !left.is_zero())
    }

    /// The query to send now: the draft, once typing has paused, when it
    /// differs from the last query sent. Each change is sent once.
    pub fn due(&mut self, now: Instant) -> Option<LogQuery> {
        if self.wait(now).is_some() {
            return None;
        }
        self.typed = None;
        if self.sent.as_ref() == Some(&self.draft) {
            return None;
        }
        Some(self.send(None))
    }

    /// A full load (first paint, Reload, history changed): the draft, now.
    pub fn reload(&mut self) -> LogQuery {
        self.typed = None;
        self.send(None)
    }

    /// Go to Hash for a commit that is not loaded: load the draft with `rev`.
    pub fn find(&mut self, rev: String) -> LogQuery {
        self.typed = None;
        self.send(Some(rev))
    }

    /// Go to Hash picked the loaded commit `hash`: scroll to it. An older
    /// Go to Hash that still loads no longer selects its commit.
    pub fn picked(&mut self, hash: &str) {
        self.finding = None;
        self.goto_note = None;
        self.reveal = Some(hash.to_owned());
    }

    /// A new query; the notes of older ones go.
    fn send(&mut self, goto: Option<String>) -> LogQuery {
        self.generation = next_generation();
        self.sent = Some(self.draft.clone());
        self.loading = true;
        self.error = None;
        self.goto_note = None;
        self.finding.clone_from(&goto);
        LogQuery {
            filter: self.draft.clone(),
            goto,
            generation: self.generation,
        }
    }

    /// Takes a load's reply. `showing` says that a good log is on screen.
    pub fn answer(&mut self, log: &CommitLog, showing: bool) -> Answer {
        if log.query.generation != self.generation {
            return Answer::Stale;
        }
        self.loading = false;
        // False when a newer Go to Hash picked a loaded commit meanwhile.
        let finding = self.finding.take().is_some();
        self.learn(log);
        if finding {
            self.goto_note.clone_from(&log.missing);
        }
        if let Some(error) = &log.error
            && showing
        {
            self.error = Some(error.clone());
            return Answer::KeepShown;
        }
        self.error = None;
        let found = log.found.clone().filter(|_| finding);
        self.reveal.clone_from(&found);
        Answer::Show(found)
    }

    /// The query numbered `generation` ended without a reply (cancelled or
    /// failed). False when a newer query was sent.
    pub fn ended(&mut self, generation: u64, error: Option<String>) -> bool {
        if generation != self.generation {
            return false;
        }
        self.loading = false;
        self.finding = None;
        if error.is_some() {
            self.error = error;
        }
        true
    }

    /// Keeps the menu choices of a loaded log. Authors add up, so a User
    /// filter does not hide the other users.
    fn learn(&mut self, log: &CommitLog) {
        self.authors
            .extend(log.commits.iter().map(|commit| commit.author.clone()));
        if log.error.is_some() {
            return;
        }
        let mut locals = Vec::new();
        let mut remotes = Vec::new();
        for entry in &log.refs {
            let list = match entry.kind {
                RefKind::LocalBranch => &mut locals,
                // `origin/HEAD` names another remote branch.
                RefKind::RemoteBranch if !entry.name.ends_with("/HEAD") => &mut remotes,
                _ => continue,
            };
            list.push((entry.full_name.clone(), entry.name.clone()));
        }
        locals.append(&mut remotes);
        self.branches = locals;
        self.me.clone_from(&log.me);
    }
}
