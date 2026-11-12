//! Settings sections, grouped by subject, and the text a search matches
//! for each one.
use crate::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingsSection {
    Appearance,
    Terminal,
    Editor,
    Languages,
    Git,
    Browser,
    Keymap,
    Notifications,
    Agents,
    /// Older deep link to the hooks part of Agents; it opens Agents.
    AgentHooks,
    Remote,
    History,
    Updates,
}

impl SettingsSection {
    /// The sections in nav order. [`Self::AgentHooks`] is only an alias.
    pub const ALL: [Self; 12] = [
        Self::Appearance,
        Self::Terminal,
        Self::Editor,
        Self::Languages,
        Self::Git,
        Self::Browser,
        Self::Keymap,
        Self::Notifications,
        Self::Agents,
        Self::Remote,
        Self::History,
        Self::Updates,
    ];

    /// The section that shows: an alias shows its owner.
    pub(crate) const fn canonical(self) -> Self {
        match self {
            Self::AgentHooks => Self::Agents,
            section => section,
        }
    }

    pub fn title(self) -> &'static str {
        match self.canonical() {
            Self::Appearance => "Appearance",
            Self::Terminal => "Terminal",
            Self::Editor => "Editor",
            Self::Languages => "Languages",
            Self::Git => "Git & Diff",
            Self::Browser => "Browser",
            Self::Keymap => "Keymap",
            Self::Notifications => "Notifications",
            Self::Agents | Self::AgentHooks => "Agents",
            Self::Remote => "Remote",
            Self::History => "History",
            Self::Updates => "Updates",
        }
    }

    /// Older fixture ids that still name this section's nav row.
    /// "Terminal & Editor" held the editor mode and external editor rows,
    /// which are now in Editor.
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn fixture_aliases(self) -> &'static [&'static str] {
        match self {
            Self::Editor => &["Terminal & Editor"],
            Self::Agents => &["Agent Hooks"],
            Self::Remote => &["SSH"],
            Self::Keymap => &["Shortcuts"],
            _ => &[],
        }
    }

    pub(super) fn icon(self) -> &'static str {
        match self.canonical() {
            Self::Appearance => "Eye",
            Self::Terminal => "Terminal",
            Self::Editor => "Pencil",
            Self::Languages => "FileBraces",
            Self::Git => "GitBranch",
            Self::Browser => "AppWindow",
            Self::Keymap => "Keyboard",
            Self::Notifications => "Bell",
            Self::Agents | Self::AgentHooks => "AgentGeneric",
            Self::Remote => "Globe",
            Self::History => "History",
            Self::Updates => "Download",
        }
    }

    /// Every row label and keyword of the section, so a search finds the
    /// section when one of its rows matches. A test paints each section and
    /// checks that its rows are here.
    pub(crate) fn keywords(self) -> &'static str {
        match self.canonical() {
            Self::Appearance => {
                "theme preset dark contrast accent selection density compact comfortable spacing \
                 terminal font family monospace nerd powerline cjk fallback font chinese japanese \
                 korean reset color hex interface window surface hover border text main button \
                 primary background foreground git added modified deleted agent status running \
                 waiting failed width pane divider"
            }
            Self::Terminal => {
                "shell override zsh bash fish sh font size type terminal type-ahead typeahead \
                 prediction latency ssh echo folder access privacy files folders"
            }
            Self::Editor => {
                "editor mode embedded neovim nvim terminal external native editor executable \
                 vscode cursor zed rustrover custom binary test draft launch font size text points \
                 line spacing height leading caret cursor blink brackets rainbow bracket pair \
                 colorization nesting vim keybindings modal inlay hints types parameters lsp \
                 problems error lens inline diagnostics autosave auto save focus delay \
                 documentation quick hover mouse pointer docs"
            }
            Self::Languages => {
                "lsp language server path spell checking spelling typo harper dictionary"
            }
            Self::Git => {
                "git diff viewer native neovim review default layout unified split side by side \
                 editor close timeout seconds delay quit pull requests github gh metadata"
            }
            Self::Browser => {
                "browser search engine address bar url query developer tools devtools inspector \
                 web inspect"
            }
            Self::Keymap => {
                "keymap jetbrains intellij shortcuts keys double shift find action go to file \
                 command shortcut chord palette"
            }
            Self::Notifications => {
                "events waiting permission completed failed ntfy channel machine phone push alert \
                 os desktop dismiss notifications focus resolve manual position corner terminal osc \
                 test place notifications at the side play sound with desktop notifications"
            }
            Self::Agents | Self::AgentHooks => {
                "agents presets preset launcher program arguments new agent session terminal \
                 installed path detected hooks claude codex opencode muse grok cursor install hook \
                 hello test verify notification"
            }
            Self::Remote => {
                "ssh remote host keep-alive keepalive serveraliveinterval idle drop keep shared \
                 connection reuse controlpersist multiplex master connect timeout connecttimeout \
                 unreachable command probe slow sign-in login detached vnc sessions minutes \
                 disconnect"
            }
            Self::History => {
                "history days age retain mib per session total disk scrollback local snapshots \
                 files revert size limit budget"
            }
            Self::Updates => {
                "updates check automatic sparkle install installation repair session service"
            }
        }
    }

    /// The text a search matches for the section: its title, its keywords,
    /// and its generated rows (language servers, keymap actions).
    pub(super) fn haystack(self) -> String {
        let mut text = format!("{} {}", self.title(), self.keywords());
        match self.canonical() {
            Self::Languages => {
                for family in terminator_native_edit::lsp::SERVER_FAMILIES {
                    text.push(' ');
                    text.push_str(family.id);
                    text.push(' ');
                    text.push_str(family.label);
                    for candidate in family.candidates {
                        text.push(' ');
                        text.push_str(candidate);
                    }
                }
            }
            Self::Keymap => {
                for action in Action::ALL {
                    text.push(' ');
                    text.push_str(action.label());
                    text.push(' ');
                    text.push_str(action.id());
                }
            }
            _ => {}
        }
        text
    }
}
