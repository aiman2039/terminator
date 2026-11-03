//! The unnamed register and its sync with the system clipboard
//! (`clipboard=unnamed`).
use super::helpers::normalize_newlines;
use super::state::VimEngine;

impl VimEngine {
    /// Adopt system clipboard text as the unnamed register before `p`/`P`
    /// (`clipboard=unnamed`): text copied in another app pastes. Text the
    /// editor itself yanked keeps its linewise/blockwise shape.
    pub fn adopt_clipboard(&mut self, text: &str) {
        // A yank staged this frame has not reached the clipboard yet: the
        // register is newer than anything the clipboard holds.
        if self.clipboard.is_some() {
            return;
        }
        let text = normalize_newlines(text).into_owned();
        if text.is_empty() || self.clipboard_seen.as_deref() == Some(text.as_str()) {
            return;
        }
        self.clipboard_seen = Some(text.clone());
        // Some clipboards add a final newline to a linewise yank.
        if self.register.linewise && text.strip_suffix('\n') == Some(self.register.text.as_str()) {
            return;
        }
        let (text, linewise) = match text.strip_suffix('\n') {
            Some(lines) => (lines.to_owned(), true),
            None => (text, false),
        };
        self.register.text = text;
        self.register.linewise = linewise;
        self.register.block_width = None;
    }

    /// Write the unnamed register and stage the text for host clipboard
    /// sync. All register writes funnel through here — never assign the
    /// fields directly (except invalidation, which writes nothing).
    pub(super) fn set_register(
        &mut self,
        text: String,
        linewise: bool,
        block_width: Option<usize>,
    ) {
        if !text.is_empty() {
            self.clipboard = Some(text.clone());
            self.clipboard_seen = Some(text.clone());
            self.clipboard_snapshot = false;
        }
        self.register.text = text;
        self.register.linewise = linewise;
        self.register.block_width = block_width;
    }

    /// Write a charwise unnamed register without touching the system
    /// clipboard (`d`, `c`, `x`, `s` with a motion), so text copied in
    /// another app survives a small edit.
    pub(super) fn set_register_local(&mut self, text: String) {
        self.register.text = text;
        self.register.linewise = false;
        self.register.block_width = None;
        // With a yank still staged, `clipboard_seen` already holds what
        // the clipboard will contain; otherwise ask the host for it.
        self.clipboard_snapshot = self.clipboard.is_none();
    }

    /// The host should read the clipboard and pass it to
    /// [`Self::note_clipboard`] (a delete just wrote only the register).
    #[must_use]
    pub fn take_clipboard_snapshot(&mut self) -> bool {
        std::mem::take(&mut self.clipboard_snapshot)
    }

    /// The system clipboard text right after a register-only delete: `p`
    /// keeps the register until the clipboard changes from this.
    pub fn note_clipboard(&mut self, text: Option<&str>) {
        self.clipboard_seen = text.map(|text| normalize_newlines(text).into_owned());
    }

    /// Take staged clipboard text, if a yank or register write happened
    /// since the last take. Pure host plumbing.
    #[must_use]
    pub fn take_clipboard(&mut self) -> Option<String> {
        self.clipboard.take()
    }
}
