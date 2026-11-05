//! Completion answers beyond the item list: the server's trigger
//! characters, and `completionItem/resolve` for the docs panel and for
//! auto-import edits that arrive after an item was accepted.
use super::*;

/// Resolve answers kept for views that have not drawn since (a hidden
/// tab); the oldest go first.
const MAX_RESOLVED: usize = 16;

/// Completion proposals for one request (`generation` drops stale ones).
pub struct CompletionResults {
    pub path: PathBuf,
    pub generation: u64,
    pub items: Vec<L::CompletionItem>,
    /// The server wants to be asked again as the user types
    /// (`isIncomplete`), not only filtered locally.
    pub incomplete: bool,
}

/// What a `completionItem/resolve` answer is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolveFor {
    /// Item `index` of the popup list that request `generation` answered:
    /// its docs and detail.
    Popup { generation: u64, index: usize },
    /// An item accepted before its auto-import edits were known. They
    /// apply only while the text before char column `col` of `line`
    /// still hashes to `before`.
    Accepted {
        line: usize,
        col: usize,
        before: u64,
    },
}

/// One `completionItem/resolve` answer, taken by the editor.
pub struct ResolvedCompletion {
    pub path: PathBuf,
    pub target: ResolveFor,
    pub item: L::CompletionItem,
}

impl LspManager {
    /// Characters that open completion as they are typed in `path` (the
    /// server's `triggerCharacters`). Empty while it starts or lists none.
    pub fn completion_triggers(&self, path: &Path) -> Vec<char> {
        self.ready_server(path)
            .map(|server| L::completion_triggers(&server.capabilities))
            .unwrap_or_default()
    }

    /// Ask for the lazy parts of `item` (the server's own JSON): docs,
    /// detail, and auto-import edits. False sends nothing: no ready server
    /// tracks `path`, or it does not resolve items.
    pub fn request_completion_resolve(
        &mut self,
        path: &Path,
        item: &serde_json::Value,
        target: ResolveFor,
    ) -> bool {
        let resolves = self
            .ready_server(path)
            .is_some_and(|server| L::completion_resolves(&server.capabilities));
        resolves
            && self.send_for(
                path,
                |id, _| L::completion_resolve(id, item),
                Pending::CompletionResolve(path.to_owned(), target),
            )
    }

    /// Resolve answers that arrived for `path`, oldest first.
    pub fn take_resolved_completions(&mut self, path: &Path) -> Vec<ResolvedCompletion> {
        let (taken, kept) = std::mem::take(&mut self.completion_resolved)
            .into_iter()
            .partition(|answer| answer.path == path);
        self.completion_resolved = kept;
        taken
    }

    /// Keep a resolve answer for its editor. A failed answer is dropped:
    /// the docs panel stays as listed, and an accepted item keeps what it
    /// already inserted. So is an answer for a closed file: its edits
    /// must not land in the file opened again.
    pub(super) fn on_completion_resolve(
        &mut self,
        path: PathBuf,
        target: ResolveFor,
        answer: Answer,
    ) {
        if !self.docs.contains_key(&path) {
            return;
        }
        let Some(item) = answer.result.as_ref().and_then(L::parse_completion_item) else {
            return;
        };
        if self.completion_resolved.len() >= MAX_RESOLVED {
            self.completion_resolved.remove(0);
        }
        self.completion_resolved
            .push(ResolvedCompletion { path, target, item });
    }
}

#[cfg(test)]
#[path = "completion_tests.rs"]
mod tests;
