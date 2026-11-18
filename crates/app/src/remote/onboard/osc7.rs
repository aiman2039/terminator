//! The lines a remote shell needs so the Explorer can follow the terminal
//! (OSC 7). Terminator only shows and copies them: it never writes to the
//! host.
use crate::appearance;
use eframe::egui::{self, RichText};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Shell {
    #[default]
    Zsh,
    Bash,
    Fish,
    Tmux,
}

impl Shell {
    const ALL: [Self; 4] = [Self::Zsh, Self::Bash, Self::Fish, Self::Tmux];

    /// The shell named by `$SHELL`'s basename; zsh when unknown.
    pub(crate) fn from_name(name: &str) -> Self {
        match name {
            "bash" | "sh" => Self::Bash,
            "fish" => Self::Fish,
            _ => Self::Zsh,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Zsh => "zsh",
            Self::Bash => "bash",
            Self::Fish => "fish",
            Self::Tmux => "tmux",
        }
    }

    /// Where the lines go on the host.
    pub(crate) fn file(self) -> &'static str {
        match self {
            Self::Zsh => "~/.zshrc",
            Self::Bash => "~/.bashrc",
            Self::Fish => "~/.config/fish/config.fish",
            Self::Tmux => "~/.tmux.conf",
        }
    }

    pub(crate) fn snippet(self) -> &'static str {
        match self {
            Self::Zsh => {
                "__terminator_osc7() { print -n \"\\e]7;file://${HOST}${PWD}\\a\"; }\nautoload -Uz add-zsh-hook\nadd-zsh-hook precmd __terminator_osc7"
            }
            Self::Bash => {
                "__terminator_osc7() { printf '\\e]7;file://%s%s\\a' \"$HOSTNAME\" \"$PWD\"; }\nPROMPT_COMMAND=\"__terminator_osc7${PROMPT_COMMAND:+;$PROMPT_COMMAND}\""
            }
            Self::Fish => {
                "# fish 4 sends this by itself; older fish needs:\nfunction __terminator_osc7 --on-variable PWD\n    printf '\\e]7;file://%s%s\\a' (hostname) \"$PWD\"\nend"
            }
            Self::Tmux => {
                "set -g allow-passthrough on\n# and in the shell function above, inside tmux:\n# printf '\\ePtmux;\\e\\e]7;file://%s%s\\a\\e\\\\' \"$HOSTNAME\" \"$PWD\""
            }
        }
    }
}

/// Tabs per shell, the lines for the chosen one, and Copy. `found` is the
/// login shell the host reported, if any.
pub(crate) fn card(ui: &mut egui::Ui, shell: &mut Shell, found: Option<&str>) {
    ui.horizontal(|ui| {
        for choice in Shell::ALL {
            let label =
                if found.is_some_and(|f| Shell::from_name(f) == choice && choice != Shell::Tmux) {
                    format!("{} (found)", choice.label())
                } else {
                    choice.label().to_owned()
                };
            if ui.selectable_label(*shell == choice, label).clicked() {
                *shell = choice;
            }
        }
    });
    ui.weak(format!("Add to {} on the host:", shell.file()));
    let snippet = shell.snippet();
    egui::Frame::new()
        .fill(ui.visuals().extreme_bg_color)
        .corner_radius(egui::CornerRadius::same(appearance::RADIUS_MEDIUM))
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(snippet)
                    .monospace()
                    .size(appearance::FONT_SMALL),
            );
        });
    ui.horizontal(|ui| {
        if ui.button("Copy").clicked() {
            ui.ctx().copy_text(snippet.to_owned());
        }
        ui.weak("Then open a new terminal. Mosh terminals do not send folder reports.");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippets_send_osc7_with_host_and_folder() {
        for shell in [Shell::Zsh, Shell::Bash, Shell::Fish] {
            let text = shell.snippet();
            assert!(text.contains("]7;file://"), "{text}");
            assert!(text.contains("PWD"), "{text}");
        }
        assert!(Shell::Tmux.snippet().contains("allow-passthrough on"));
    }

    #[test]
    fn login_shell_picks_the_tab() {
        assert_eq!(Shell::from_name("bash"), Shell::Bash);
        assert_eq!(Shell::from_name("fish"), Shell::Fish);
        assert_eq!(Shell::from_name("zsh"), Shell::Zsh);
        assert_eq!(Shell::from_name(""), Shell::Zsh);
    }
}
