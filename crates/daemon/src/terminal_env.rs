use portable_pty::CommandBuilder;
use std::ffi::OsStr;

/// Match Orca: an automation parent's color suppression must not leak into
/// interactive PTYs. Shell startup files can still set their own preferences.
pub fn restore_colors(command: &mut CommandBuilder) {
    command.env_remove("NO_COLOR");
    for key in ["FORCE_COLOR", "CLICOLOR"] {
        if command.get_env(key) == Some(OsStr::new("0")) {
            command.env_remove(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interactive_terminals_remove_inherited_color_suppression() {
        for no_color in ["1", "true", "0", ""] {
            let mut command = CommandBuilder::new("sh");
            command.env_clear();
            command.env("NO_COLOR", no_color);
            command.env("FORCE_COLOR", "0");
            command.env("CLICOLOR", "0");
            command.env("TERM", "xterm-256color");
            command.env("COLORTERM", "truecolor");
            restore_colors(&mut command);
            for key in ["NO_COLOR", "FORCE_COLOR", "CLICOLOR"] {
                assert_eq!(command.get_env(key), None, "{key}");
            }
            assert_eq!(command.get_env("TERM"), Some(OsStr::new("xterm-256color")));
            assert_eq!(command.get_env("COLORTERM"), Some(OsStr::new("truecolor")));
        }
    }

    #[test]
    fn interactive_terminals_preserve_other_color_preferences_and_environment() {
        for value in [None, Some(""), Some("1"), Some("2"), Some("3")] {
            let mut command = CommandBuilder::new("sh");
            command.env_clear();
            if let Some(value) = value {
                command.env("FORCE_COLOR", value);
                command.env("CLICOLOR", value);
            }
            command.env("CLICOLOR_FORCE", "1");
            command.env("UNRELATED_SETTING", "preserved");
            restore_colors(&mut command);
            assert_eq!(command.get_env("FORCE_COLOR"), value.map(OsStr::new));
            assert_eq!(command.get_env("CLICOLOR"), value.map(OsStr::new));
            assert_eq!(command.get_env("CLICOLOR_FORCE"), Some(OsStr::new("1")));
            assert_eq!(
                command.get_env("UNRELATED_SETTING"),
                Some(OsStr::new("preserved"))
            );
        }
    }
}
