//! Keeps a Settings draft on top of the live settings. When the daemon's
//! settings change under an open or hidden Settings session, the user's
//! own edits stay and every other value follows the new settings, so
//! Apply never writes a stale value back.
use crate::*;
use serde_json::{Map, Value};

/// `draft` moved from `base` onto `live`. A value the user changed
/// (`draft` differs from `base`) wins; any other value comes from `live`.
/// Objects merge key by key.
pub(super) fn rebase(base: &Settings, draft: &Settings, live: &Settings) -> Settings {
    let (Ok(base), Ok(draft), Ok(next)) = (
        serde_json::to_value(base),
        serde_json::to_value(draft),
        serde_json::to_value(live),
    ) else {
        return live.clone();
    };
    merge(Some(&base), Some(&draft), Some(&next))
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_else(|| live.clone())
}

fn merge(base: Option<&Value>, draft: Option<&Value>, live: Option<&Value>) -> Option<Value> {
    if let (Some(Value::Object(base)), Some(Value::Object(draft)), Some(Value::Object(live))) =
        (base, draft, live)
    {
        let mut out = Map::new();
        for key in base.keys().chain(draft.keys()).chain(live.keys()) {
            if !out.contains_key(key)
                && let Some(value) = merge(base.get(key), draft.get(key), live.get(key))
            {
                out.insert(key.clone(), value);
            }
        }
        return Some(Value::Object(out));
    }
    if draft == base {
        live.cloned()
    } else {
        draft.cloned()
    }
}

impl App {
    /// Called before `live` replaces `state.settings`.
    pub(crate) fn rebase_settings_draft(&mut self, live: &Settings) {
        if !self.settings_session || &self.state.settings == live {
            return;
        }
        let editor = self.settings_draft.external_editor.clone();
        self.settings_draft = rebase(&self.state.settings, &self.settings_draft, live);
        if self.settings_draft.external_editor != editor {
            self.editor_preset = external_editor::selected(&self.settings_draft);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::rebase;
    use terminator_core::Settings;

    #[test]
    fn rebase_keeps_user_edits_and_takes_newer_live_values() {
        let base = Settings::default();
        let mut draft = base.clone();
        draft.shell = "/bin/zsh".into();
        let mut live = base.clone();
        live.spell_check = !base.spell_check;
        let merged = rebase(&base, &draft, &live);
        assert_eq!(merged.shell, "/bin/zsh");
        assert_eq!(merged.spell_check, live.spell_check);
    }

    #[test]
    fn rebase_of_an_untouched_draft_is_the_live_settings() {
        let base = Settings::default();
        let mut live = base.clone();
        live.shell = "/bin/fish".into();
        live.lsp_servers.insert("rust".into(), "ra".into());
        assert_eq!(rebase(&base, &base, &live), live);
    }
}
