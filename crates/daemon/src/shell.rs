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
                if file == ".zshrc" {
                    content += &prompt_script("zsh", &hook);
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
            let content = content + &prompt_script("bash", &hook);
            let rc = dir.join("bashrc");
            atomic_write(&rc, content.as_bytes())?;
            cmd.arg("--rcfile");
            cmd.arg(rc);
            cmd.arg("-i");
        }
        "fish" => {
            cmd.arg("-i");
            cmd.arg("-C");
            cmd.arg(format!("function _terminator_cwd --on-event fish_prompt; {hook} cwd \"$PWD\" >/dev/null 2>&1; end; {}", prompt_script("fish", &hook)));
        }
        _ => {
            cmd.arg("-i");
        }
    }
    Ok(cmd)
}

fn prompt_script(shell: &str, hook: &str) -> String {
    match shell {
        "zsh" => format!(
            r#"
_terminator_epoch=0
_terminator_command() {{ _terminator_epoch=$({hook} prompt begin 2>/dev/null); }}
_terminator_prompt() {{
    [[ -o promptsubst || -n "${{widgets[zle-line-init]-}}" || "${{precmd_functions[-1]}}" != _terminator_prompt ]] && return
    {hook} prompt "$_terminator_epoch" "$(jobs -p)" >/dev/null 2>&1
}}
add-zsh-hook preexec _terminator_command
add-zsh-hook precmd _terminator_prompt
"#
        ),
        "bash" => format!(
            r#"
# An existing DEBUG trap owns command instrumentation. Preserve it and fall back to confirmation.
if [[ -z "$(trap -p DEBUG)" && $- != *T* ]]; then
    _terminator_epoch=0
    _terminator_command() {{ case "$1" in _terminator_*) ;; *) _terminator_epoch=$({hook} prompt begin 2>/dev/null);; esac; }}
    _terminator_prompt() {{
        [[ "$PS1" == *'$('* || "$PS1" == *'${{'* || "$PS1" == *'`'* ]] && return
        {hook} prompt "$_terminator_epoch" "$(jobs -pr; jobs -ps)" >/dev/null 2>&1
    }}
    if declare -p PROMPT_COMMAND 2>/dev/null | command grep -q 'declare -a'; then
        PROMPT_COMMAND+=(_terminator_prompt)
    else
        PROMPT_COMMAND="${{PROMPT_COMMAND:+$PROMPT_COMMAND; }}_terminator_prompt"
    fi
    trap '_terminator_command "$BASH_COMMAND"' DEBUG
fi
"#
        ),
        "fish" => format!(
            r#"
set -g _terminator_epoch 0
function _terminator_command --on-event fish_preexec
    set -g _terminator_epoch ({hook} prompt begin 2>/dev/null)
end
# The fish_prompt event precedes rendering, so acknowledge only after the
# original prompt function returns. If it cannot be copied, retain confirmation.
type -q fish_prompt
if functions -c fish_prompt _terminator_original_prompt
    function fish_prompt
        _terminator_original_prompt
        set -l prompt_status $status
        if not functions -q fish_right_prompt
            set -l prompt_jobs (jobs -p)
            if test (count $prompt_jobs) -eq 0
                {hook} prompt "$_terminator_epoch" "" >/dev/null 2>&1
            else
                {hook} prompt "$_terminator_epoch" "busy" >/dev/null 2>&1
            end
        end
        return $prompt_status
    end
end
"#
        ),
        _ => String::new(),
    }
}
