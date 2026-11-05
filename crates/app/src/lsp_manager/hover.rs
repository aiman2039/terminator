//! `textDocument/hover` for Quick Documentation. Answers are kept by the
//! spot they describe (file, buffer revision, line and word), so a
//! pointer back on a word asks nothing again, and an answer for another
//! spot or older text never shows for this one.
use super::*;
use std::sync::Arc;

/// Answers kept at most; the oldest go first.
const KEEP: usize = 64;
/// A request still unanswered after this long may be sent again (its
/// server may have died with it).
const ASK_AGAIN: Duration = Duration::from_secs(10);

/// What one hover answer describes: char columns `start..end` of `line`
/// in `path` at buffer `revision`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HoverKey {
    pub(crate) path: PathBuf,
    pub(crate) revision: u64,
    pub(crate) line: usize,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

enum Entry {
    Asked(Instant),
    /// The docs, or `None` when the server has nothing there. A server
    /// still loading answers `None` too, so that answer is asked again.
    Answered(Option<Arc<L::Hover>>),
    /// The request failed ("content modified" while indexing): shown as
    /// no docs, and asked again next time.
    Failed,
}

/// Hover answers and requests in flight, newest last.
#[derive(Default)]
pub(super) struct HoverCache {
    entries: Vec<(HoverKey, Entry)>,
}

impl HoverCache {
    fn get(&self, key: &HoverKey) -> Option<&Entry> {
        self.entries
            .iter()
            .rev()
            .find(|(known, _)| known == key)
            .map(|(_, entry)| entry)
    }

    fn put(&mut self, key: HoverKey, entry: Entry) {
        self.entries.retain(|(known, _)| *known != key);
        self.entries.push((key, entry));
        if self.entries.len() > KEEP {
            self.entries.remove(0);
        }
    }

    /// Forget the answers about a closed file.
    pub(super) fn forget(&mut self, path: &Path) {
        self.entries.retain(|(key, _)| key.path != path);
    }
}

impl LspManager {
    /// Whether the docs for `key` are known or on their way.
    pub(crate) fn hover_asked(&self, key: &HoverKey) -> bool {
        match self.hover.get(key) {
            Some(Entry::Answered(docs)) => docs.is_some(),
            Some(Entry::Asked(since)) => since.elapsed() < ASK_AGAIN,
            Some(Entry::Failed) | None => false,
        }
    }

    /// Ask for the docs at `position` (UTF-16), answered under `key`. The
    /// buffer text must already be with the server. `Ok(false)` when no
    /// server tracks the file or its server has no hover; `Err` says why
    /// its server cannot answer now (missing, starting, not trusted).
    pub(crate) fn request_hover(
        &mut self,
        key: &HoverKey,
        position: L::Position,
    ) -> Result<bool, String> {
        if self.hover_asked(key) {
            return Ok(true);
        }
        if let Some(note) = self.server_status(&key.path) {
            return Err(note);
        }
        if !self.server_supports(&key.path, "hoverProvider") {
            return Ok(false);
        }
        let sent = self.send_for(
            &key.path,
            |id, uri| L::hover(id, uri, position),
            Pending::Hover(key.clone()),
        );
        if sent {
            self.hover.put(key.clone(), Entry::Asked(Instant::now()));
        }
        Ok(sent)
    }

    /// The docs for `key`: `None` while they are on their way (or were
    /// never asked for), `Some(None)` when there are none.
    #[expect(
        clippy::option_option,
        reason = "Not answered yet and answered with no docs are both needed."
    )]
    pub(crate) fn hover_docs(&self, key: &HoverKey) -> Option<Option<Arc<L::Hover>>> {
        match self.hover.get(key)? {
            Entry::Asked(_) => None,
            Entry::Answered(docs) => Some(docs.clone()),
            Entry::Failed => Some(None),
        }
    }

    pub(super) fn on_hover(&mut self, key: HoverKey, answer: Answer) {
        let entry = if answer.error.is_some() {
            Entry::Failed
        } else {
            Entry::Answered(L::parse_hover(&answer.value()).map(Arc::new))
        };
        self.hover.put(key, entry);
    }
}
