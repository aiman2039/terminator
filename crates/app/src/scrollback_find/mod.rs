//! Search all scrollback: one text (case toggle, optional regex) over the
//! saved scrollback of every session of the selected project, or of all
//! projects. A replaceable background job reads each session with the
//! existing `History` request and matches on the CPU pool; the GUI thread
//! only submits it and takes the result. The work is bounded: sessions,
//! bytes read, time, and result rows. Agent conversation files are not
//! read.
mod job;
mod page;

use crate::App;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use terminator_core::async_service::CancellationToken;

/// Sessions searched at most, newest first.
pub(crate) const MAX_SESSIONS: usize = 200;
/// Result rows kept at most.
pub(crate) const MAX_RESULTS: usize = 500;
/// Scrollback bytes read at most per search.
pub(crate) const MAX_BYTES: usize = 128 * 1024 * 1024;
/// A search stops after this long and shows what it found.
pub(crate) const TIME_BUDGET: Duration = Duration::from_secs(20);
/// Characters of a matching line shown.
pub(crate) const LINE_CHARS: usize = 240;

/// What to look for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Query {
    pub(crate) text: String,
    pub(crate) match_case: bool,
    pub(crate) regex: bool,
}

/// A compiled query.
#[derive(Clone, Debug)]
pub(crate) enum Matcher {
    Plain { needle: String, match_case: bool },
    Regex(regex::Regex),
}

impl Matcher {
    pub(crate) fn new(query: &Query) -> Result<Self, String> {
        if query.text.is_empty() {
            return Err("Type something to search for.".into());
        }
        if query.regex {
            return regex::RegexBuilder::new(&query.text)
                .case_insensitive(!query.match_case)
                .size_limit(1 << 20)
                .build()
                .map(Self::Regex)
                .map_err(|error| format!("Invalid regular expression: {error}"));
        }
        let needle = if query.match_case {
            query.text.clone()
        } else {
            query.text.to_lowercase()
        };
        Ok(Self::Plain {
            needle,
            match_case: query.match_case,
        })
    }

    pub(crate) fn is_match(&self, line: &str) -> bool {
        match self {
            Self::Plain { needle, match_case } if *match_case => line.contains(needle.as_str()),
            Self::Plain { needle, .. } => line.to_lowercase().contains(needle.as_str()),
            Self::Regex(regex) => regex.is_match(line),
        }
    }
}

/// One matching line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Hit {
    pub(crate) session: String,
    /// 0-based line in the session's scrollback text.
    pub(crate) line: usize,
    pub(crate) text: String,
}

/// A finished search.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Found {
    pub(crate) hits: Vec<Hit>,
    pub(crate) searched: usize,
    pub(crate) failed: usize,
    /// More rows matched than [`MAX_RESULTS`].
    pub(crate) truncated: bool,
    /// The time or byte budget ran out before every session was read.
    pub(crate) stopped: bool,
}

/// Matching lines of `text`, at most `limit`, each cut to
/// [`LINE_CHARS`]. Returns the hits and whether more matched.
pub(crate) fn scan_text(
    session: &str,
    text: &str,
    matcher: &Matcher,
    limit: usize,
) -> (Vec<Hit>, bool) {
    let mut hits = Vec::new();
    for (line, body) in text.lines().enumerate() {
        if !matcher.is_match(body) {
            continue;
        }
        if hits.len() >= limit {
            return (hits, true);
        }
        hits.push(Hit {
            session: session.to_owned(),
            line,
            text: body.chars().take(LINE_CHARS).collect(),
        });
    }
    (hits, false)
}

type Slot = Arc<Mutex<Option<(u64, Result<Found, String>)>>>;

/// Data of `CenterPage::ScrollbackFind`.
#[derive(Default)]
pub(crate) struct FindPage {
    pub(crate) query: Query,
    /// Search all projects, not only the selected one.
    pub(crate) all_projects: bool,
    pub(crate) generation: u64,
    pub(crate) running: bool,
    pub(crate) result: Option<Result<Found, String>>,
    /// The query the result answers.
    pub(crate) searched: Query,
    pub(crate) focus: bool,
    slot: Slot,
    cancel: Option<CancellationToken>,
}

impl FindPage {
    /// Takes a finished job's result; an older generation is dropped.
    pub(crate) fn poll(&mut self) {
        let done = self
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some((generation, result)) = done
            && generation == self.generation
        {
            self.result = Some(result);
            self.running = false;
            self.cancel = None;
        }
    }

    /// Stops the running job, if any.
    pub(crate) fn stop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.cancel();
        }
        self.running = false;
    }
}

impl App {
    pub(crate) fn open_scrollback_find(&mut self) {
        self.show_center(crate::CenterPage::ScrollbackFind);
        self.scrollback_find.focus = true;
    }

    /// Sessions to search, newest first, at most [`MAX_SESSIONS`].
    pub(crate) fn scrollback_find_sessions(&self, all_projects: bool) -> Vec<String> {
        let project = self.selected_project().map(|p| p.id.clone());
        let mut sessions: Vec<&terminator_core::Session> = self
            .state
            .sessions
            .iter()
            .filter(|s| !s.review && (all_projects || Some(&s.project_id) == project.as_ref()))
            .collect();
        sessions.sort_by_key(|s| std::cmp::Reverse(s.created));
        sessions
            .into_iter()
            .take(MAX_SESSIONS)
            .map(|s| s.id.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests;
