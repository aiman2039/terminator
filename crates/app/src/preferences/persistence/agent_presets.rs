//! Agent presets: named agent commands kept with the UI preferences.
use serde::{Deserialize, Serialize};

/// Recency entries kept for the New agent session picker.
const RECENT_LIMIT: usize = 20;

/// A named command that opens an agent in a new terminal. `kind` is a
/// catalog agent kind for the brand icon; any other value shows the
/// generic agent icon.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentPreset {
    pub name: String,
    pub kind: String,
    pub program: String,
    pub args: Vec<String>,
}

impl AgentPreset {
    /// The shell text typed into the terminal: the program and each
    /// argument quoted by `shlex`. `None` when the program is blank or a
    /// word cannot be quoted (a NUL byte).
    pub fn command(&self) -> Option<String> {
        let program = self.program.trim();
        if program.is_empty() {
            return None;
        }
        shlex::try_join(std::iter::once(program).chain(self.args.iter().map(String::as_str))).ok()
    }
}

impl super::UiPreferences {
    /// Puts launcher `key` first in the recency list.
    pub fn note_agent_launch(&mut self, key: &str) {
        self.agent_launch_recent.retain(|known| known != key);
        self.agent_launch_recent.insert(0, key.to_owned());
        self.agent_launch_recent.truncate(RECENT_LIMIT);
    }
}

#[cfg(test)]
mod tests {
    use super::super::UiPreferences;
    use super::AgentPreset;

    fn preset(program: &str, args: &[&str]) -> AgentPreset {
        AgentPreset {
            name: "Fast".into(),
            kind: "claude".into(),
            program: program.into(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        }
    }

    #[test]
    fn preset_command_quotes_arguments_for_the_shell() {
        let command = preset("claude", &["--model", "opus 4", "it's"])
            .command()
            .unwrap();
        assert_eq!(command, r#"claude --model 'opus 4' "it's""#);
        assert_eq!(
            shlex::split(&command).unwrap(),
            ["claude", "--model", "opus 4", "it's"]
        );
        assert_eq!(preset("codex", &[]).command().as_deref(), Some("codex"));
    }

    #[test]
    fn preset_without_program_has_no_command() {
        assert_eq!(preset("  ", &["--yes"]).command(), None);
        assert_eq!(preset("claude", &["a\0b"]).command(), None);
    }

    #[test]
    fn agent_presets_default_to_empty_and_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ui-preferences.json"),
            r#"{"version":2,"agent_presets":[{"name":"Old","program":"codex"}]}"#,
        )
        .unwrap();
        let legacy = UiPreferences::load(dir.path()).unwrap();
        assert_eq!(legacy.agent_presets[0].args, Vec::<String>::new());
        assert_eq!(legacy.agent_presets[0].kind, "");
        assert!(UiPreferences::default().agent_presets.is_empty());
        let mut prefs = legacy;
        prefs.agent_presets.push(preset("claude", &["--resume"]));
        prefs.note_agent_launch("agent:codex");
        prefs.save(dir.path()).unwrap();
        let loaded = UiPreferences::load(dir.path()).unwrap();
        assert_eq!(loaded.agent_presets, prefs.agent_presets);
        assert_eq!(loaded.agent_launch_recent, ["agent:codex"]);
    }

    #[test]
    fn latest_launch_moves_to_the_front_once() {
        let mut prefs = UiPreferences::default();
        prefs.note_agent_launch("agent:codex");
        prefs.note_agent_launch("preset:Fast");
        prefs.note_agent_launch("agent:codex");
        assert_eq!(prefs.agent_launch_recent, ["agent:codex", "preset:Fast"]);
    }
}
