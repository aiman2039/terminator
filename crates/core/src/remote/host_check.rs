//! What a login-directory probe found on the host: the login shell and the
//! tools Terminator uses there. Read-only; the probe installs nothing.
use serde::{Deserialize, Serialize};

/// Tools the probe looks for, in report order.
pub const TOOLS: [&str; 3] = ["git", "nvim", "mosh-server"];

/// Longest version text kept per tool.
const MAX_VERSION: usize = 80;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HostTools {
    /// Basename of the login shell (`$SHELL`), for example `zsh`. Empty
    /// when the host did not say.
    pub shell: String,
    pub tools: Vec<HostTool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HostTool {
    pub name: String,
    /// First line of `--version`; empty when the tool is missing.
    pub version: String,
    pub found: bool,
}

impl HostTools {
    #[must_use]
    pub fn has(&self, name: &str) -> bool {
        self.tools.iter().any(|t| t.name == name && t.found)
    }
}

/// POSIX script that prints one `shell NAME` line, then one
/// `tool NAME 0|1 VERSION` line per [`TOOLS`] entry. It only reads.
#[must_use]
pub fn script() -> String {
    let mut script = String::from("printf 'shell %s\\n' \"${SHELL##*/}\";");
    for tool in TOOLS {
        script.push_str(&format!(
            " if command -v {tool} >/dev/null 2>&1; then printf 'tool {tool} 1 %s\\n' \"$({tool} --version 2>/dev/null | head -n 1)\"; else printf 'tool {tool} 0\\n'; fi;"
        ));
    }
    script
}

/// Parses [`script`] output. Unknown lines and tools are ignored.
#[must_use]
pub fn parse(text: &str) -> HostTools {
    let mut found = HostTools::default();
    for line in text.lines() {
        if let Some(shell) = line.strip_prefix("shell ") {
            found.shell = clean(shell);
            continue;
        }
        let Some(rest) = line.strip_prefix("tool ") else {
            continue;
        };
        let mut parts = rest.splitn(3, ' ');
        let (Some(name), Some(flag)) = (parts.next(), parts.next()) else {
            continue;
        };
        if !TOOLS.contains(&name) || found.tools.iter().any(|t| t.name == name) {
            continue;
        }
        found.tools.push(HostTool {
            name: name.to_owned(),
            found: flag == "1",
            version: clean(parts.next().unwrap_or_default()),
        });
    }
    found
}

/// Printable text, at most [`MAX_VERSION`] characters.
fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(MAX_VERSION)
        .collect::<String>()
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_output_lists_shell_and_tools() {
        let found = parse(
            "shell zsh\ntool git 1 git version 2.43.0\ntool nvim 0\ntool mosh-server 1 mosh-server (mosh 1.4.0)\ntool rm 1 x\n",
        );
        assert_eq!(found.shell, "zsh");
        assert!(found.has("git") && found.has("mosh-server"));
        assert!(!found.has("nvim") && !found.has("rm"));
        assert_eq!(found.tools[0].version, "git version 2.43.0");
    }

    #[test]
    fn hostile_output_is_bounded_and_printable() {
        let long = "x".repeat(500);
        let found = parse(&format!("shell \u{1b}[31mzsh\ntool git 1 {long}\n"));
        assert_eq!(found.shell, "[31mzsh");
        assert_eq!(found.tools[0].version.len(), MAX_VERSION);
    }

    #[test]
    fn script_names_every_tool() {
        let script = script();
        for tool in TOOLS {
            assert!(script.contains(&format!("command -v {tool}")), "{script}");
        }
    }
}
