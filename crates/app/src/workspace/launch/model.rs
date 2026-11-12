//! The launch file: tabs, splits, per-pane folders and optional commands.
//!
//! ```toml
//! [[tab]]
//! name = "Dev"            # optional tab name
//!
//! [[tab.pane]]            # the first pane fills the new tab
//! cwd = "web"             # optional, relative to the project folder
//! command = "npm run dev" # optional; typed when the prompt shows
//!
//! [[tab.pane]]
//! split = "right"         # "right" (default) or "down" of the previous pane
//! command = "cargo watch"
//! enter = false           # optional; false only types the command
//! ```
//!
//! A command that names an agent program is typed and never sent; a
//! shell command follows the task Enter rule (Enter unless `enter =
//! false`). The project file comes from the repository, so it needs the
//! project trust answer before anything opens.
use crate::PostCreate;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path};

/// Project launch file, relative to the project folder.
pub(crate) const LAUNCH_FILE: &str = ".terminator/layout.toml";
/// Panes per launch; a larger file is refused.
pub(crate) const MAX_PANES: usize = 32;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LaunchFile {
    #[serde(default, rename = "tab")]
    pub(crate) tabs: Vec<LaunchTab>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LaunchTab {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    #[serde(default, rename = "pane")]
    pub(crate) panes: Vec<LaunchPane>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LaunchPane {
    /// Where this pane goes beside the previous one; the first pane
    /// ignores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) split: Option<Split>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) enter: Option<bool>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Split {
    #[default]
    Right,
    Down,
}

impl Split {
    /// The split name the create requests use.
    pub(crate) fn request(self) -> &'static str {
        match self {
            Self::Right => "right",
            Self::Down => "down",
        }
    }
}

/// The file's text, checked: at least one pane, at most [`MAX_PANES`],
/// and every folder inside the project.
pub(crate) fn parse(text: &str) -> Result<LaunchFile, String> {
    let mut file: LaunchFile = toml_edit::de::from_str(text).map_err(|e| e.to_string())?;
    file.tabs.retain(|tab| !tab.panes.is_empty());
    let panes: usize = file.tabs.iter().map(|tab| tab.panes.len()).sum();
    if panes == 0 {
        return Err("The launch file has no [[tab.pane]] entries.".into());
    }
    if panes > MAX_PANES {
        return Err(format!("The launch file has more than {MAX_PANES} panes."));
    }
    for pane in file.tabs.iter_mut().flat_map(|tab| tab.panes.iter_mut()) {
        pane.cwd = pane
            .cwd
            .take()
            .map(|cwd| cwd.trim().to_owned())
            .filter(|cwd| !cwd.is_empty());
        pane.command = pane
            .command
            .take()
            .map(|command| command.trim().to_owned())
            .filter(|command| !command.is_empty());
        if let Some(cwd) = &pane.cwd
            && !inside_project(Path::new(cwd))
        {
            return Err(format!(
                "Pane folder \"{cwd}\" must stay inside the project."
            ));
        }
        if pane
            .command
            .as_deref()
            .is_some_and(|c| c.contains(['\n', '\r']))
        {
            return Err("A pane command must be one line.".into());
        }
    }
    Ok(file)
}

fn inside_project(path: &Path) -> bool {
    path.components()
        .all(|part| matches!(part, Component::Normal(_) | Component::CurDir))
}

/// The file text for `file`.
pub(crate) fn render(file: &LaunchFile) -> Result<String, String> {
    let body = toml_edit::ser::to_string_pretty(file).map_err(|e| e.to_string())?;
    Ok(format!(
        "# Terminator launch layout: Find Action \"Open launch layout\" opens it.\n\
         # Each [[tab.pane]] after the first splits \"right\" or \"down\" of the previous one.\n\n{body}"
    ))
}

/// What a pane types once its prompt shows. An agent command is typed
/// and never sent; a shell command is sent unless `enter = false`.
pub(crate) fn pane_input(pane: &LaunchPane, agent_words: &[String]) -> Option<PostCreate> {
    let command = pane.command.clone()?;
    let enter = pane.enter.unwrap_or(true) && !crate::tasks::names_agent(&command, agent_words);
    Some(PostCreate {
        input: command,
        enter,
    })
}

/// Whether the trust answer lets a file from `repository` open anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Gate {
    Open,
    NeedsTrust,
    NotTrusted,
}

pub(crate) fn gate(from_repository: bool, trust: Option<bool>) -> Gate {
    match (from_repository, trust) {
        (false, _) | (true, Some(true)) => Gate::Open,
        (true, None) => Gate::NeedsTrust,
        (true, Some(false)) => Gate::NotTrusted,
    }
}
