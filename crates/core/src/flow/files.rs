//! The flow files of a project: `NAME.toml` or the drawing
//! `NAME.excalidraw` in `.terminator/flows/`. The runner and the GUI find
//! and read flows only through this module. Blocking: call it off the GUI
//! thread.
use super::{FLOW_DIR, Flow, FlowError, drawing};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("A flow name must be letters, digits, '-' or '_'")]
    Name,
    #[error("No flow file {0}.toml or {0}.excalidraw in .terminator/flows")]
    Missing(String),
    #[error("{0}.toml and {0}.excalidraw are both there; keep one")]
    Twice(String),
    #[error("A flow with the name {0} is there already")]
    Exists(String),
    #[error("Cannot write {0}: {1}")]
    Write(PathBuf, std::io::Error),
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

fn is_flow_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext == "toml" || ext == drawing::EXTENSION)
}

/// The names of the project's flow files, sorted. A name with both a TOML
/// file and a drawing shows once; [`load`] then says which to keep.
pub fn names(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root.join(FLOW_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| is_flow_file(path))
        .filter_map(|path| Some(path.file_stem()?.to_string_lossy().into_owned()))
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// The file of flow `name`: the TOML file or the drawing.
pub fn path_of(root: &Path, name: &str) -> Result<PathBuf, LoadError> {
    if !plain_name(name) {
        return Err(LoadError::Name);
    }
    let folder = root.join(FLOW_DIR);
    let toml = folder.join(format!("{name}.toml"));
    let drawn = folder.join(format!("{name}.{}", drawing::EXTENSION));
    match (toml.is_file(), drawn.is_file()) {
        (true, true) => Err(LoadError::Twice(name.to_owned())),
        (true, false) => Ok(toml),
        (false, true) => Ok(drawn),
        (false, false) => Err(LoadError::Missing(name.to_owned())),
    }
}

/// Read and check flow `name` of the project at `root`.
pub fn load(root: &Path, name: &str) -> Result<Flow, LoadError> {
    let path = path_of(root, name)?;
    let text =
        std::fs::read_to_string(&path).map_err(|error| LoadError::Read(path.clone(), error))?;
    if path
        .extension()
        .is_some_and(|ext| ext == drawing::EXTENSION)
    {
        Ok(drawing::parse_drawing(&text, name)?)
    } else {
        Ok(super::parse(&text)?)
    }
}

/// Makes the drawing of a new flow `name` from the template and gives its
/// path. It never replaces a file.
pub fn new_drawing(root: &Path, name: &str) -> Result<PathBuf, LoadError> {
    let path = new_drawing_path(root, name)?;
    let write = || -> std::io::Result<()> {
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        std::io::Write::write_all(&mut file, drawing::template().as_bytes())
    };
    write().map_err(|error| LoadError::Write(path.clone(), error))?;
    Ok(path)
}

/// Where a new drawn flow `name` goes. Refused when a flow with that name
/// is there.
pub fn new_drawing_path(root: &Path, name: &str) -> Result<PathBuf, LoadError> {
    match path_of(root, name) {
        Err(LoadError::Missing(_)) => Ok(root
            .join(FLOW_DIR)
            .join(format!("{name}.{}", drawing::EXTENSION))),
        Ok(_) | Err(LoadError::Twice(_)) => Err(LoadError::Exists(name.to_owned())),
        Err(error) => Err(error),
    }
}
