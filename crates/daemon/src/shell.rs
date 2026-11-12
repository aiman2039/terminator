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
            let original = std::env::var("ZDOTDIR")
                .unwrap_or_else(|_| std::env::var("HOME").unwrap_or_default());
            write_zsh_wrappers(&dir, &original, &hook)?;
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

/// Each wrapper sources the user's file from the user's ZDOTDIR, then points
/// ZDOTDIR back at `dir` so zsh reads the next wrapper. A ZDOTDIR that the
/// user's own files set (for example `~/.zshenv` exporting an XDG path) is
/// kept in `_terminator_zdotdir` for the later files and the final value.
fn write_zsh_wrappers(dir: &Path, original: &str, hook: &str) -> Result<()> {
    for file in [".zshenv", ".zprofile", ".zshrc", ".zlogin"] {
        let mut content = format!(
            "[[ -n \"$_terminator_zdotdir\" ]] || _terminator_zdotdir={}\nZDOTDIR=\"$_terminator_zdotdir\"\n[[ -f \"$ZDOTDIR/{file}\" ]] && source \"$ZDOTDIR/{file}\"\n_terminator_zdotdir=\"$ZDOTDIR\"\n",
            quote(original)
        );
        if file == ".zshrc" {
            content += &format!(
                "autoload -Uz add-zsh-hook\n_terminator_cwd() {{ {hook} cwd \"$PWD\" >/dev/null 2>&1; }}\nadd-zsh-hook precmd _terminator_cwd\nadd-zsh-hook chpwd _terminator_cwd\n"
            );
            content += crate::shell_integration::ZSH;
        }
        // zsh reads subsequent startup files using ZDOTDIR; keep the user's after zlogin.
        if file != ".zlogin" {
            content += &format!("ZDOTDIR={}\n", quote(&dir.to_string_lossy()));
        }
        atomic_write(&dir.join(file), content.as_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zsh_loads_the_rc_from_a_zdotdir_set_in_the_users_zshenv() {
        let home = tempfile::tempdir().unwrap();
        let config = home.path().join("cfg");
        let wrappers = home.path().join("wrappers");
        fs::create_dir_all(&config).unwrap();
        fs::create_dir_all(&wrappers).unwrap();
        fs::write(
            home.path().join(".zshenv"),
            "export ZDOTDIR=\"$HOME/cfg\"\n",
        )
        .unwrap();
        fs::write(config.join(".zshrc"), "USER_RC=loaded\n").unwrap();
        let home_text = home.path().to_string_lossy();
        write_zsh_wrappers(&wrappers, &home_text, "true").unwrap();
        let Ok(output) = std::process::Command::new("zsh")
            .args(["-l", "-i", "-c", "print -r -- \"$USER_RC|$ZDOTDIR\""])
            .env_clear()
            .env("HOME", home.path())
            .env("ZDOTDIR", &wrappers)
            .env("TERM", "dumb")
            .stdin(std::process::Stdio::null())
            .output()
        else {
            return; // No zsh on this machine.
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            stdout.lines().last().unwrap_or_default(),
            format!("loaded|{}", config.to_string_lossy())
        );
    }
}
