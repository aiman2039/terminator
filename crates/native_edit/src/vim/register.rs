//! Registers: the unnamed one and its sync with the system clipboard
//! (`clipboard=unnamed`), named, yank, small-delete and black-hole.
use super::helpers::normalize_newlines;
use super::mode::Register;
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
        } else if linewise {
            // One blank line: keep it over older clipboard text, like a
            // register-only delete does.
            self.clipboard_snapshot = self.clipboard.is_none();
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

/// Where a register write goes, from the `"x` prefix.
enum Target {
    Unnamed,
    BlackHole,
    Clipboard,
    Named(char),
}

impl VimEngine {
    fn target(&self) -> Target {
        match self.x.reg_sel {
            None | Some('"') => Target::Unnamed,
            Some('_') => Target::BlackHole,
            Some('+' | '*') => Target::Clipboard,
            Some(c) => Target::Named(c),
        }
    }

    /// A yank (`y`, `yy`, Visual `y`): the selected register, else the
    /// unnamed one (synced to the clipboard) and `"0`.
    pub(super) fn write_yank(&mut self, text: String, linewise: bool, block_width: Option<usize>) {
        let reg = Register {
            text: text.clone(),
            linewise,
            block_width,
        };
        match self.target() {
            Target::BlackHole => {}
            Target::Named(c) => self.write_named(c, reg),
            Target::Unnamed | Target::Clipboard => {
                self.x.registers.insert('0', reg);
                self.set_register(text, linewise, block_width);
            }
        }
    }

    /// A delete or change: linewise and blockwise deletes sync the
    /// clipboard like a yank; small charwise ones stay local (`"-`).
    pub(super) fn write_delete(&mut self, text: String, linewise: bool) {
        self.write_delete_shaped(text, linewise, None);
    }

    pub(super) fn write_delete_shaped(
        &mut self,
        text: String,
        linewise: bool,
        block_width: Option<usize>,
    ) {
        match self.target() {
            Target::BlackHole => {}
            Target::Named(c) => self.write_named(
                c,
                Register {
                    text,
                    linewise,
                    block_width,
                },
            ),
            Target::Clipboard => self.set_register(text, linewise, block_width),
            Target::Unnamed if linewise || block_width.is_some() => {
                self.set_register(text, linewise, block_width);
            }
            Target::Unnamed => {
                self.x.registers.insert(
                    '-',
                    Register {
                        text: text.clone(),
                        linewise: false,
                        block_width: None,
                    },
                );
                self.set_register_local(text);
            }
        }
    }

    /// `"a`-`"z` (uppercase appends), `"0`-`"9`, `"-`; the unnamed
    /// register follows the write without touching the clipboard.
    fn write_named(&mut self, name: char, mut reg: Register) {
        let key = name.to_ascii_lowercase();
        if name.is_ascii_uppercase()
            && let Some(old) = self.x.registers.get(&key)
        {
            let linewise = old.linewise || reg.linewise;
            let sep = if linewise { "\n" } else { "" };
            reg = Register {
                text: format!("{}{sep}{}", old.text, reg.text),
                linewise,
                block_width: None,
            };
        }
        self.register = reg.clone();
        self.clipboard_snapshot = self.clipboard.is_none();
        self.x.registers.insert(key, reg);
    }

    /// The register `p`/`P` read: the selected one, else the unnamed one
    /// (which already adopted the clipboard for `"+`). `None` when empty;
    /// a linewise register of one blank line is not empty.
    pub(super) fn put_source(&self) -> Option<Register> {
        let reg = match self.x.reg_sel {
            None | Some('"' | '+' | '*') => self.register.clone(),
            Some('_') => return None,
            Some(c) => self.x.registers.get(&c.to_ascii_lowercase())?.clone(),
        };
        (reg.linewise || !reg.text.is_empty()).then_some(reg)
    }
}
