//! Saved host commands (`ssh-commands-v1`): the terminal command and the
//! `mosh-server` command of an SSH project.
use super::{RemoteError, RemoteProject};

/// [`RemoteProject::shell_command`] and [`RemoteProject::mosh_command`] are
/// stored and honored.
pub const COMMANDS_CAPABILITY: &str = "ssh-commands-v1";
/// The login shell. csh/tcsh accept `-l` only as the sole flag; a TTY makes
/// them interactive anyway.
pub const DEFAULT_SHELL_COMMAND: &str = "s=\"${SHELL:-/bin/sh}\"; case \"${s##*/}\" in csh|tcsh) exec \"$s\" -l;; esac; exec \"$s\" -il";
pub const MAX_COMMAND_LEN: usize = 4096;

/// A saved host command: blank or unchanged from `default` is `None`.
#[must_use]
pub fn normalize_command(text: &str, default: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty() && text != default.trim()).then(|| text.to_owned())
}

/// Each saved command is one trimmed, non-empty line.
pub(super) fn validate_commands(project: &RemoteProject) -> Result<(), RemoteError> {
    for command in [&project.shell_command, &project.mosh_command]
        .into_iter()
        .flatten()
    {
        let valid = !command.is_empty()
            && command.trim() == command
            && command.len() <= MAX_COMMAND_LEN
            && !command.chars().any(char::is_control);
        if !valid {
            return Err(RemoteError::InvalidCommand {
                command: command.clone(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_commands_are_optional_single_lines_and_default_is_not_saved() {
        let legacy = r#"{"id":"a","name":"A","ssh":{"host":"h"},"root":"/r"}"#;
        let mut p: RemoteProject = serde_json::from_str(legacy).unwrap();
        assert_eq!(
            (p.shell_command.clone(), p.mosh_command.clone()),
            (None, None)
        );
        let json = serde_json::to_string(&p).unwrap();
        assert!(!json.contains("shell_command") && !json.contains("mosh_command"));
        p.shell_command = Some("tmux new -A -s main".into());
        p.mosh_command = Some("mosh-server new -p 60001".into());
        assert!(p.validate().is_ok());
        let back: RemoteProject =
            serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
        for bad in ["", " tmux", "a\nb", &"x".repeat(MAX_COMMAND_LEN + 1)] {
            p.shell_command = Some(bad.to_owned());
            assert!(p.validate().is_err(), "{bad:?}");
        }
        assert_eq!(normalize_command("  ", "x"), None);
        assert_eq!(normalize_command(" x ", "x"), None);
        assert_eq!(normalize_command(" tmux ", "x").as_deref(), Some("tmux"));
    }
}
