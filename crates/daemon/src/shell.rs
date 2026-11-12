use anyhow::Result;
use portable_pty::CommandBuilder;
use std::{fs, path::Path};
use terminator_core::*;
pub fn prepare(paths: &Paths, shell: &str, helper: &Path) -> Result<CommandBuilder> {
    let dir = paths.data.join("shell");
    fs::create_dir_all(&dir)?;
    let hook = quote(&helper.to_string_lossy());
    let name = Path::new(shell)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let mut cmd = CommandBuilder::new(shell);
    match name.as_ref() {
        "zsh" => {
            let original = std::env::var("ZDOTDIR").ok();
            write_zsh_wrappers(&dir, original.as_deref(), &hook)?;
            cmd.env("ZDOTDIR", &dir);
            cmd.args(["-l", "-i"]);
        }
        "bash" => {
            let content = format!(
                "[[ -f ~/.bashrc ]] && source ~/.bashrc\n_terminator_cwd() {{ {hook} cwd \"$PWD\" >/dev/null 2>&1; }}\nif declare -p PROMPT_COMMAND 2>/dev/null | command grep -q 'declare -a'; then PROMPT_COMMAND+=(_terminator_cwd); else PROMPT_COMMAND=\"${{PROMPT_COMMAND:+$PROMPT_COMMAND; }}_terminator_cwd\"; fi\n{}",
                crate::shell_integration::BASH
            );
            let rc = dir.join("bashrc");
            atomic_write(&rc, content.as_bytes())?;
            cmd.arg("--rcfile");
            cmd.arg(rc);
            cmd.arg("-i");
        }
        "fish" => {
            cmd.arg("-i");
            cmd.arg("-C");
            cmd.arg(format!(
                "function _terminator_cwd --on-event fish_prompt; {hook} cwd \"$PWD\" >/dev/null 2>&1; end\n{}",
                crate::shell_integration::FISH
            ));
        }
        _ => {
            cmd.arg("-i");
        }
    }
    Ok(cmd)
}

/// ZDOTDIR as the user's files last left it: unset, or its value and
/// whether it is exported (`_terminator_zdotdir_type` is zsh's `${(t)…}`).
const ZSH_RESTORE_ZDOTDIR: &str = "if [[ -z \"$_terminator_zdotdir_type\" ]]; then unset ZDOTDIR; else ZDOTDIR=\"$_terminator_zdotdir\"; if [[ \"$_terminator_zdotdir_type\" == *export* ]]; then export ZDOTDIR; else typeset +x ZDOTDIR; fi; fi\n";
const ZSH_SAVE_ZDOTDIR: &str =
    "_terminator_zdotdir=\"${ZDOTDIR-}\"\n_terminator_zdotdir_type=\"${(t)ZDOTDIR}\"\n";

/// Each wrapper sources the user's file with ZDOTDIR as a plain zsh would
/// have it (unset when the daemon had none, so `${ZDOTDIR:-…}` in the
/// user's `.zshenv` works), then points ZDOTDIR back at `dir` so zsh reads
/// the next wrapper. A ZDOTDIR that the user's own files set is kept for
/// the later files and as the final value.
fn write_zsh_wrappers(dir: &Path, original: Option<&str>, hook: &str) -> Result<()> {
    for file in [".zshenv", ".zprofile", ".zshrc", ".zlogin"] {
        let mut content = String::new();
        if file == ".zshenv" {
            content += &match original {
                Some(original) => format!(
                    "_terminator_zdotdir={}\n_terminator_zdotdir_type=scalar-export\n",
                    quote(original)
                ),
                None => "_terminator_zdotdir=\n_terminator_zdotdir_type=\n".to_owned(),
            };
        }
        content += ZSH_RESTORE_ZDOTDIR;
        content += &format!(
            "[[ -f \"${{ZDOTDIR:-$HOME}}/{file}\" ]] && source \"${{ZDOTDIR:-$HOME}}/{file}\"\n"
        );
        content += ZSH_SAVE_ZDOTDIR;
        if file == ".zshrc" {
            content += &format!(
                "autoload -Uz add-zsh-hook\n_terminator_cwd() {{ {hook} cwd \"$PWD\" >/dev/null 2>&1; }}\nadd-zsh-hook precmd _terminator_cwd\nadd-zsh-hook chpwd _terminator_cwd\n"
            );
            content += crate::shell_integration::ZSH;
        }
        // zsh reads subsequent startup files using ZDOTDIR; keep the user's after zlogin.
        if file == ".zlogin" {
            content += "unset _terminator_zdotdir _terminator_zdotdir_type\n";
        } else {
            content += &format!("ZDOTDIR={}\n", quote(&dir.to_string_lossy()));
        }
        atomic_write(&dir.join(file), content.as_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run a login zsh through the wrappers with `zshenv` as the user's
    /// `~/.zshenv`, and `.zshrc` files in `$HOME/cfg` and `$HOME`. Prints
    /// which rc loaded and the final ZDOTDIR (`-` when unset). `None` when
    /// this machine has no zsh.
    fn run_zsh(zshenv: &str, daemon_zdotdir: Option<&str>) -> Option<(String, String)> {
        let home = tempfile::tempdir().unwrap();
        let config = home.path().join("cfg");
        let wrappers = home.path().join("wrappers");
        fs::create_dir_all(&config).unwrap();
        fs::create_dir_all(&wrappers).unwrap();
        fs::write(home.path().join(".zshenv"), zshenv).unwrap();
        fs::write(config.join(".zshrc"), "USER_RC=cfg\n").unwrap();
        fs::write(home.path().join(".zshrc"), "USER_RC=home\n").unwrap();
        let daemon = daemon_zdotdir.map(|dir| home.path().join(dir).to_string_lossy().into_owned());
        write_zsh_wrappers(&wrappers, daemon.as_deref(), "true").unwrap();
        let output = std::process::Command::new("zsh")
            .args(["-l", "-i", "-c", "print -r -- \"$USER_RC|${ZDOTDIR--}\""])
            .env_clear()
            .env("HOME", home.path())
            .env("ZDOTDIR", &wrappers)
            .env("TERM", "dumb")
            .stdin(std::process::Stdio::null())
            .output()
            .ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let line = stdout.lines().last().unwrap_or_default();
        let home_text = home.path().to_string_lossy().into_owned();
        let (rc, zdotdir) = line.split_once('|').unwrap_or((line, ""));
        Some((rc.to_owned(), zdotdir.replace(&home_text, "~")))
    }

    #[test]
    fn zsh_loads_the_rc_from_a_zdotdir_set_in_the_users_zshenv() {
        let cases = [
            ("export ZDOTDIR=\"$HOME/cfg\"\n", None, ("cfg", "~/cfg")),
            (
                "export ZDOTDIR=\"${ZDOTDIR:-$HOME/cfg}\"\n",
                None,
                ("cfg", "~/cfg"),
            ),
            (": ${ZDOTDIR:=$HOME/cfg}\n", None, ("cfg", "~/cfg")),
            ("", None, ("home", "-")),
            // The daemon's own ZDOTDIR is what a plain zsh would start with.
            (
                "export ZDOTDIR=\"${ZDOTDIR:-$HOME/elsewhere}\"\n",
                Some("cfg"),
                ("cfg", "~/cfg"),
            ),
        ];
        for (zshenv, daemon, (rc, zdotdir)) in cases {
            let Some(result) = run_zsh(zshenv, daemon) else {
                return; // No zsh on this machine.
            };
            assert_eq!(result, (rc.to_owned(), zdotdir.to_owned()), "{zshenv:?}");
        }
    }
}
