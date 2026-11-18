//! The flow files of a project: `NAME.toml` in `.terminator/flows/`. The
//! runner and the GUI find, read and write flows only through this module.
//! Blocking: call it off the GUI thread.
use super::{Draft, FLOW_DIR, Flow, FlowError};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("A flow name must be letters, digits, '-' or '_'")]
    Name,
    #[error("No flow file {0}.toml in .terminator/flows")]
    Missing(String),
    #[error("A flow with the name {0} is there already")]
    Exists(String),
    #[error("Cannot write {0}: {1}")]
    Write(PathBuf, String),
    #[error("Cannot read {0}: {1}")]
    Read(PathBuf, std::io::Error),
    #[error(transparent)]
    Flow(#[from] FlowError),
}

/// A flow name is a plain file name and one safe shell word, never a path.
pub fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

/// The names of the project's flow files, sorted.
pub fn names(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root.join(FLOW_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .filter_map(|path| Some(path.file_stem()?.to_string_lossy().into_owned()))
        .collect();
    names.sort_unstable();
    names
}

/// The file of flow `name`, there or not.
pub fn path_of(root: &Path, name: &str) -> Result<PathBuf, LoadError> {
    if !plain_name(name) {
        return Err(LoadError::Name);
    }
    Ok(root.join(FLOW_DIR).join(format!("{name}.toml")))
}

/// Read and check flow `name` of the project at `root`.
pub fn load(root: &Path, name: &str) -> Result<Flow, LoadError> {
    let path = path_of(root, name)?;
    if !path.is_file() {
        return Err(LoadError::Missing(name.to_owned()));
    }
    let text =
        std::fs::read_to_string(&path).map_err(|error| LoadError::Read(path.clone(), error))?;
    Ok(super::parse(&text)?)
}

/// Write `draft` as flow `name`. Only a correct flow is written. `replace`
/// is false for a new flow: a file with that name is then never replaced.
pub fn save(root: &Path, name: &str, draft: &Draft, replace: bool) -> Result<PathBuf, LoadError> {
    let path = path_of(root, name)?;
    let text = draft.to_toml()?;
    if !replace && path.exists() {
        return Err(LoadError::Exists(name.to_owned()));
    }
    let written = || -> Result<(), String> {
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
        }
        crate::atomic_write(&path, text.as_bytes()).map_err(|error| error.to_string())
    };
    written().map_err(|error| LoadError::Write(path.clone(), error))?;
    Ok(path)
}
