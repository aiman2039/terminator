//! Spell checker (`harper-ls`) settings and merged code-action state.
use super::*;

/// Code actions asked of both the language and the spell server: the
/// list is published once every asked server answered.
pub(super) struct ActionWait {
    pub(super) path: PathBuf,
    pub(super) generation: u64,
    pub(super) remaining: usize,
    pub(super) items: Vec<L::CodeActionItem>,
}

impl LspManager {
    /// Apply Settings → Check spelling. Turning it off closes every spell
    /// document; the spell servers stop on the next prune.
    pub fn set_spell_enabled(&mut self, enabled: bool) {
        if self.spell_enabled == enabled {
            return;
        }
        self.spell_enabled = enabled;
        if !enabled {
            let paths: Vec<PathBuf> = self.spell_docs.keys().cloned().collect();
            for path in &paths {
                self.close_spell(path);
            }
        }
    }
}
