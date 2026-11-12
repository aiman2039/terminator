//! Task files: named shell commands for a project.
//!
//! Two TOML files hold tasks, both in this format:
//!
//! ```toml
//! [[task]]
//! name = "Test"
//! command = "cargo test"
//! cwd = "crates/app"   # optional, relative to the project folder
//! enter = false        # optional; false only types the command
//! ```
//!
//! The user file (`tasks.toml` next to `config.toml`) applies to every
//! project. The project file (`.terminator/tasks.toml`) comes from the
//! repository, so none of its commands is typed before the project is
//! trusted. A task runs only on an explicit Run.
use crate::PostCreate;
use std::path::{Component, Path, PathBuf};

/// Project task file, relative to the project folder.
pub(crate) const PROJECT_TASK_FILE: &str = ".terminator/tasks.toml";
/// User task file name, in the folder of `config.toml`.
pub(crate) const USER_TASK_FILE: &str = "tasks.toml";

/// Where a task was defined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TaskSource {
    /// The user's own file: no trust question.
    User,
    /// The repository's file: needs the project trust answer.
    Project,
}

impl TaskSource {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::User => "User",
            Self::Project => "Project",
        }
    }
}

/// One named command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TaskDef {
    pub(crate) name: String,
    pub(crate) command: String,
    /// Working folder relative to the project folder.
    pub(crate) cwd: Option<String>,
    /// Press Enter after typing (default true; see [`plan`]).
    pub(crate) enter: bool,
    pub(crate) source: TaskSource,
}

#[derive(serde::Deserialize)]
struct TaskFile {
    #[serde(default)]
    task: Vec<RawTask>,
}

#[derive(serde::Deserialize)]
struct RawTask {
    name: String,
    command: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default = "enter_default")]
    enter: bool,
}

fn enter_default() -> bool {
    true
}

/// Tasks in one file's text. Tasks without a name or command are left
/// out; a project task whose folder leaves the project is an error.
pub(crate) fn parse_tasks(text: &str, source: TaskSource) -> Result<Vec<TaskDef>, String> {
    let file: TaskFile = toml_edit::de::from_str(text).map_err(|error| error.to_string())?;
    let mut tasks = Vec::new();
    for raw in file.task {
        let (name, command) = (raw.name.trim(), raw.command.trim());
        if name.is_empty() || command.is_empty() {
            continue;
        }
        let cwd = raw
            .cwd
            .map(|cwd| cwd.trim().to_owned())
            .filter(|cwd| !cwd.is_empty());
        if source == TaskSource::Project
            && let Some(cwd) = &cwd
            && !inside_project(Path::new(cwd))
        {
            return Err(format!("Task \"{name}\": cwd must stay inside the project"));
        }
        tasks.push(TaskDef {
            name: name.to_owned(),
            command: command.to_owned(),
            cwd,
            enter: raw.enter,
            source,
        });
    }
    Ok(tasks)
}

fn inside_project(path: &Path) -> bool {
    path.components()
        .all(|part| matches!(part, Component::Normal(_) | Component::CurDir))
}

/// What one load found: tasks (user first) and file errors.
#[derive(Clone, Debug, Default)]
pub(crate) struct LoadedTasks {
    pub(crate) project: String,
    pub(crate) root: PathBuf,
    pub(crate) tasks: Vec<TaskDef>,
    pub(crate) errors: Vec<String>,
}

/// Read both task files. Blocking: call it on a worker thread. A missing
/// file is not an error.
pub(crate) fn load_tasks(
    project: String,
    root: PathBuf,
    user_file: Option<PathBuf>,
) -> LoadedTasks {
    let mut loaded = LoadedTasks {
        project,
        tasks: Vec::new(),
        errors: Vec::new(),
        root,
    };
    let project_file = loaded.root.join(PROJECT_TASK_FILE);
    let files = user_file
        .map(|path| (path, TaskSource::User))
        .into_iter()
        .chain([(project_file, TaskSource::Project)]);
    for (path, source) in files {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        match parse_tasks(&text, source) {
            Ok(tasks) => loaded.tasks.extend(tasks),
            Err(error) => loaded.errors.push(format!("{}: {error}", path.display())),
        }
    }
    loaded
}

/// What a Run click does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RunPlan {
    /// A project task before the trust answer: nothing is typed.
    NeedsTrust,
    /// A project task in a project the user did not trust.
    NotTrusted,
    /// Type `input` into a new terminal; `enter` runs it.
    Type(PostCreate),
}

/// The Enter rule. A Run click is an explicit user action, so a task's
/// shell command runs (Enter) unless the task says `enter = false` or the
/// command names an agent program anywhere: an agent command is typed and
/// never sent, as for agent presets. Project tasks need trust first.
pub(crate) fn plan(task: &TaskDef, trust: Option<bool>, agent_words: &[String]) -> RunPlan {
    if task.source == TaskSource::Project {
        match trust {
            None => return RunPlan::NeedsTrust,
            Some(false) => return RunPlan::NotTrusted,
            Some(true) => {}
        }
    }
    RunPlan::Type(PostCreate {
        input: task.command.clone(),
        enter: task.enter && !names_agent(&task.command, agent_words),
    })
}

/// Whether any word of `command` is an agent program: also a path's last
/// part (`/usr/bin/claude`), a package name (`@openai/codex`), or one part
/// of a dashed or dotted word (`claude-code`, `codex.js`). Case is
/// ignored. The test errs toward typing without Enter.
pub(crate) fn names_agent(command: &str, agent_words: &[String]) -> bool {
    let is_agent = |word: &str| {
        agent_words
            .iter()
            .any(|agent| agent.eq_ignore_ascii_case(word))
    };
    command
        .split(|c: char| !(c.is_alphanumeric() || matches!(c, '.' | '_' | '-')))
        .filter(|word| !word.is_empty())
        .any(|word| is_agent(word) || word.split(['-', '.']).any(is_agent))
}

/// Every agent program name: the catalog's executables, entry points and
/// kinds, plus the programs of the user's agent presets.
pub(crate) fn agent_words(presets: &[crate::preferences::AgentPreset]) -> Vec<String> {
    let catalog = terminator_core::agents::CATALOG.iter().flat_map(|info| {
        std::iter::once(info.kind)
            .chain(info.exes.iter().copied())
            .chain(info.entrypoints.iter().copied())
            .map(str::to_owned)
    });
    let presets = presets.iter().filter_map(|preset| {
        let program = preset.program.trim();
        let name = Path::new(program)
            .file_name()?
            .to_string_lossy()
            .into_owned();
        (!name.is_empty()).then_some(name)
    });
    let mut words: Vec<String> = catalog.chain(presets).collect();
    words.sort_unstable();
    words.dedup();
    words
}
