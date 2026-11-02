//! Shell integration scripts added to the wrapper startup files. They emit
//! OSC 133 prompt/command marks, OSC 633 `E` (command line) and `P;Cwd`.
//! Values use the 633 escaping: `\\` and `\xAB`. Set
//! `TERMINATOR_SHELL_INTEGRATION=0` to turn the marks off.

/// zsh: prepended to `precmd_functions` so it reads the command's `$?`
/// before other hooks change it.
pub(crate) const ZSH: &str = r#"if [[ "${TERMINATOR_SHELL_INTEGRATION:-1}" != 0 && -z "$_terminator_si" ]]; then
_terminator_si=1
_terminator_esc() { REPLY=${1//\\/\\\\}; REPLY=${REPLY//;/\\x3b}; REPLY=${REPLY//$'\n'/\\x0a}; REPLY=${REPLY//$'\a'/\\x07}; REPLY=${REPLY//$'\e'/\\x1b}; }
_terminator_si_precmd() {
  local ec=$?
  builtin printf '\e]133;D;%s\a' "$ec"
  _terminator_esc "$PWD"; builtin printf '\e]633;P;Cwd=%s\a\e]133;A\a' "$REPLY"
  [[ "$PS1" == *$'\e]133;B'* ]] || PS1+=$'%{\e]133;B\a%}'
}
_terminator_si_preexec() { _terminator_esc "$1"; builtin printf '\e]633;E;%s\a\e]133;C\a' "$REPLY"; }
precmd_functions=(_terminator_si_precmd $precmd_functions)
preexec_functions+=(_terminator_si_preexec)
fi
"#;

/// bash: runs first in `PROMPT_COMMAND`; `PS0` (bash 4.4+) marks the
/// command start. The command line is not sent; the daemon uses the echo.
pub(crate) const BASH: &str = r#"if [[ "${TERMINATOR_SHELL_INTEGRATION:-1}" != 0 && -z "$_terminator_si" ]]; then
_terminator_si=1
_terminator_si_prompt() {
  local ec=$? cwd=${PWD//\\/\\\\}
  cwd=${cwd//;/\\x3b}
  builtin printf '\e]133;D;%s\a\e]633;P;Cwd=%s\a\e]133;A\a' "$ec" "$cwd"
  [[ "$PS1" == *'133;B'* ]] || PS1+='\[\e]133;B\a\]'
  return $ec
}
PS0+='\e]133;C\a'
if declare -p PROMPT_COMMAND 2>/dev/null | command grep -q 'declare -a'; then PROMPT_COMMAND=(_terminator_si_prompt "${PROMPT_COMMAND[@]}"); else PROMPT_COMMAND="_terminator_si_prompt${PROMPT_COMMAND:+; $PROMPT_COMMAND}"; fi
fi
"#;

/// fish: event handlers; `fish_postexec` sees the command's `$status`.
pub(crate) const FISH: &str = r#"if test "$TERMINATOR_SHELL_INTEGRATION" != 0; and not set -q _terminator_si
set -g _terminator_si 1
function _terminator_si_esc; string replace -a '\\' '\\\\' -- $argv | string replace -a ';' '\\x3b' | string join '\\x0a'; end
function _terminator_si_prompt --on-event fish_prompt; printf '\e]633;P;Cwd=%s\a\e]133;A\a' (_terminator_si_esc $PWD); end
function _terminator_si_preexec --on-event fish_preexec; printf '\e]633;E;%s\a\e]133;C\a' (_terminator_si_esc $argv[1]); end
function _terminator_si_postexec --on-event fish_postexec; printf '\e]133;D;%s\a' $status; end
end"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// Run `script` in `shell` if it is installed; `None` when it is not.
    fn run(shell: &str, args: &[&str]) -> Option<std::process::Output> {
        std::process::Command::new(shell)
            .args(args)
            .env("TERMINATOR_SHELL_INTEGRATION", "1")
            .output()
            .ok()
    }

    #[test]
    fn zsh_hooks_emit_finish_cwd_and_escaped_command_line() {
        let script = format!(
            "{ZSH}\nfalse\n_terminator_si_precmd\n_terminator_si_preexec 'echo a;b'\nprint -rn -- \"|$PS1\""
        );
        let Some(output) = run("zsh", &["-f", "-c", &script]) else {
            return;
        };
        let out = String::from_utf8_lossy(&output.stdout);
        assert!(out.contains("\x1b]133;D;1\x07"), "{out:?}");
        assert!(out.contains("\x1b]633;P;Cwd="), "{out:?}");
        assert!(out.contains("\x1b]133;A\x07"), "{out:?}");
        assert!(
            out.contains("\x1b]633;E;echo a\\x3bb\x07\x1b]133;C\x07"),
            "{out:?}"
        );
        assert!(
            out.contains("133;B"),
            "PS1 gains the prompt-end mark: {out:?}"
        );
    }

    // Windows `bash` can be the WSL launcher, which has no distribution on CI.
    #[cfg(unix)]
    #[test]
    fn bash_prompt_command_keeps_the_exit_status() {
        let script = format!("{BASH}\nfalse\n_terminator_si_prompt\necho \"|$?|$PS0\"");
        let Some(output) = run("bash", &["--norc", "--noprofile", "-c", &script]) else {
            return;
        };
        let out = String::from_utf8_lossy(&output.stdout);
        assert!(out.contains("\x1b]133;D;1\x07"), "{out:?}");
        assert!(out.contains("|1|"), "the user's $? survives: {out:?}");
        assert!(out.contains("133;C"), "{out:?}");
    }

    #[test]
    fn integration_can_be_turned_off() {
        let script = format!("{ZSH}\nprint -rn -- \"${{_terminator_si:-off}}\"");
        let Some(output) = std::process::Command::new("zsh")
            .args(["-f", "-c", &script])
            .env("TERMINATOR_SHELL_INTEGRATION", "0")
            .output()
            .ok()
        else {
            return;
        };
        assert_eq!(String::from_utf8_lossy(&output.stdout), "off");
    }
}
