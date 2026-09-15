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
            for file in [".zshenv", ".zprofile", ".zshrc", ".zlogin"] {
                let mut content = format!(
                    "ZDOTDIR={}\n[[ -f \"$ZDOTDIR/{file}\" ]] && source \"$ZDOTDIR/{file}\"\n",
                    quote(&original)
                );
                if file == ".zshrc" {
                    content += &format!(
                        "autoload -Uz add-zsh-hook\n_terminator_cwd() {{ {hook} cwd \"$PWD\" >/dev/null 2>&1; }}\nadd-zsh-hook precmd _terminator_cwd\nadd-zsh-hook chpwd _terminator_cwd\n"
                    );
                }
                // zsh reads subsequent startup files using ZDOTDIR; restore original after zlogin.
                if file != ".zlogin" {
                    content += &format!("ZDOTDIR={}\n", quote(&dir.to_string_lossy()));
                }
                atomic_write(&dir.join(file), content.as_bytes())?;
            }
            cmd.env("ZDOTDIR", &dir);
            cmd.args(["-l", "-i"]);
        }
        "bash" => {
            let content = format!(
                "[[ -f ~/.bashrc ]] && source ~/.bashrc\n_terminator_cwd() {{ {hook} cwd \"$PWD\" >/dev/null 2>&1; }}\nif declare -p PROMPT_COMMAND 2>/dev/null | command grep -q 'declare -a'; then PROMPT_COMMAND+=(_terminator_cwd); else PROMPT_COMMAND=\"${{PROMPT_COMMAND:+$PROMPT_COMMAND; }}_terminator_cwd\"; fi\n"
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
                "function _terminator_cwd --on-event fish_prompt; {hook} cwd \"$PWD\" >/dev/null 2>&1; end"
            ));
        }
        _ => {
            cmd.arg("-i");
        }
    }
    Ok(cmd)
}
