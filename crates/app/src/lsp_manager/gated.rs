//! Requests sent only when the server advertises them: Go to
//! Implementation / Type Declaration, document highlights, and range /
//! on-type formatting. A missing capability never sends a request.
use super::*;
use std::sync::Arc;

/// Server-computed occurrences of the symbol at the caret, for one
/// buffer revision.
pub(super) struct DocHighlights {
    pub(super) path: PathBuf,
    pub(super) revision: u64,
    pub(super) ranges: Arc<Vec<L::Range>>,
}

impl LspManager {
    /// The ready server for `path`, when one tracks it.
    fn ready_server(&self, path: &Path) -> Option<&Server> {
        let doc = self.docs.get(path)?;
        self.servers.get(&doc.server).filter(|server| server.ready)
    }

    /// Whether the ready server for `path` advertises `provider`
    /// (`documentRangeFormattingProvider`, …). False while starting.
    pub fn server_supports(&self, path: &Path, provider: &str) -> bool {
        self.ready_server(path)
            .is_some_and(|server| L::supports(&server.capabilities, provider))
    }

    /// Characters that trigger on-type formatting for `path` (empty when
    /// the server does not advertise it).
    pub fn on_type_triggers(&self, path: &Path) -> Vec<char> {
        self.ready_server(path)
            .map(|server| L::on_type_triggers(&server.capabilities))
            .unwrap_or_default()
    }

    /// Go to Implementation / Type Declaration (or definition) at a
    /// UTF-16 position. The answer lands in [`take_goto`] like a
    /// definition: one target jumps, several open the chooser. Errors say
    /// why nothing was sent.
    pub fn request_locate(
        &mut self,
        path: &Path,
        position: L::Position,
        kind: L::Locate,
    ) -> Result<(), String> {
        if let Some(note) = self.server_status(path) {
            return Err(note);
        }
        let doc = self
            .docs
            .get(path)
            .ok_or("No language server tracks this file")?;
        let (key, uri) = (doc.server.clone(), doc.uri.clone());
        let server = self
            .servers
            .get_mut(&key)
            .filter(|server| server.ready)
            .ok_or("Language server is not ready")?;
        if kind != L::Locate::Definition && !L::supports(&server.capabilities, kind.provider()) {
            return Err(format!(
                "The {} language server cannot go to {}",
                key.language,
                kind.label()
            ));
        }
        let L::RequestId::Number(number) = server.ids.next_id() else {
            return Err("Language server request ids exhausted".into());
        };
        server
            .inbox
            .send(L::locate(
                kind,
                &L::RequestId::Number(number),
                &uri,
                position,
            ))
            .map_err(|_| "Language server stopped".to_owned())?;
        server.pending.insert(
            number,
            Pending::Definition {
                path: path.to_owned(),
            },
        );
        Ok(())
    }

    /// Ask for the occurrences of the symbol at `position`, once per
    /// `(path, revision, position)`. Only sent when the server advertises
    /// `documentHighlightProvider` and already holds `revision` (positions
    /// in an answer about older text would be off).
    pub fn request_document_highlight(
        &mut self,
        path: &Path,
        revision: u64,
        position: L::Position,
    ) -> bool {
        let asked = (path.to_owned(), revision, position);
        if self.highlight_asked.as_ref() == Some(&asked) {
            return false;
        }
        let synced = self
            .docs
            .get(path)
            .is_some_and(|doc| doc.opened && doc.submitted == revision);
        if !synced || !self.server_supports(path, "documentHighlightProvider") {
            return false;
        }
        self.highlight_asked = Some(asked);
        self.send_for(
            path,
            |id, uri| L::document_highlight(id, uri, position),
            Pending::DocumentHighlight {
                path: path.to_owned(),
                revision,
            },
        )
    }

    /// Server occurrences for `path` at buffer `revision`, if they arrived.
    pub fn document_highlights(&self, path: &Path, revision: u64) -> Option<Arc<Vec<L::Range>>> {
        self.highlights
            .as_ref()
            .filter(|found| found.path == path && found.revision == revision)
            .map(|found| Arc::clone(&found.ranges))
    }

    pub(super) fn on_document_highlight(
        &mut self,
        path: PathBuf,
        revision: u64,
        result: &serde_json::Value,
    ) {
        self.highlights = Some(DocHighlights {
            path,
            revision,
            ranges: Arc::new(L::parse_document_highlights(result)),
        });
    }

    /// Format `range` as of buffer `revision`, when the server advertises
    /// range formatting. False sends nothing (the caller may fall back to
    /// whole-document formatting).
    pub fn request_range_format(&mut self, path: &Path, revision: u64, range: L::Range) -> bool {
        if !self.server_supports(path, "documentRangeFormattingProvider") {
            return false;
        }
        self.send_for(
            path,
            |id, uri| L::range_formatting(id, uri, range),
            Pending::Format {
                path: path.to_owned(),
                revision,
            },
        )
    }

    /// On-type formatting after `ch` was typed, only for an advertised
    /// trigger character. The answer is dropped silently when the buffer
    /// moved on (typing continues; no "format again" note).
    pub fn request_on_type_format(
        &mut self,
        path: &Path,
        revision: u64,
        (position, ch): (L::Position, char),
    ) -> bool {
        if !self.on_type_triggers(path).contains(&ch) {
            return false;
        }
        self.send_for(
            path,
            |id, uri| L::on_type_formatting(id, uri, position, ch),
            Pending::OnTypeFormat {
                path: path.to_owned(),
                revision,
            },
        )
    }
}
