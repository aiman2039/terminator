//! GUI-only, read-only document tabs (`Tab::Document`): a file opened in a
//! native viewer with no PTY. Read on the file pool, parse on the CPU pool,
//! never in a paint callback.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};
use terminator_core::async_service::{self, CancellationToken, OperationContext, Policy};

pub mod table;
#[cfg(test)]
mod tests;
mod view;

pub use table::Table;

/// Job subsystem of table loads.
pub const SUBSYSTEM: &str = "tables";
/// Bytes read from one file; the rest is left out and the view says so.
const MAX_READ: usize = 64 * 1024 * 1024;
/// Bytes sampled to choose the CSV delimiter.
const SAMPLE: usize = 64 * 1024;
/// Open tables kept in memory at once.
const MAX_OPEN: usize = 8;

/// The viewer of a `Tab::Document`. Persisted with the path; view state
/// (sort, filter) is not.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum DocumentKind {
    Table,
}

impl DocumentKind {
    /// The default viewer for `path`, if it has one here.
    pub fn for_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?;
        ["csv", "tsv", "tab"]
            .iter()
            .any(|known| extension.eq_ignore_ascii_case(known))
            .then_some(Self::Table)
    }
}

/// Bytes of `path`, at most `MAX_READ`, and whether more were left.
/// A partial last line is dropped.
pub fn read(path: &Path, cancel: &CancellationToken) -> Result<(Vec<u8>, bool)> {
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    // Avoid blocking on FIFOs; Windows has no equivalent flag.
    #[cfg(unix)]
    options.custom_flags(libc::O_NONBLOCK);
    let file = options.open(path).context("Open table")?;
    ensure!(file.metadata()?.is_file(), "Table is not a regular file");
    let mut bytes = async_service::read_chunks(file, MAX_READ.saturating_add(1), cancel)?;
    let truncated = bytes.len() > MAX_READ;
    if truncated {
        let end = bytes.iter().rposition(|byte| *byte == b'\n').unwrap_or(0);
        bytes.truncate(end);
    }
    Ok((bytes, truncated))
}

/// Read on `fs`, parse on `cpu`.
pub async fn load(
    fs: &async_service::NativePool,
    cpu: &async_service::NativePool,
    path: PathBuf,
    cancel: &CancellationToken,
) -> Result<Table> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_owned();
    let operation = cancel.clone();
    let (bytes, truncated) = fs.run(cancel, move || read(&path, &operation)).await?;
    cpu.run(cancel, move || {
        let sample = bytes.get(..SAMPLE.min(bytes.len())).unwrap_or_default();
        let delimiter = table::delimiter(&extension, sample);
        Ok(table::parse(&bytes, delimiter, truncated))
    })
    .await
}

/// Starts a table load; its result arrives as `Update::Table`.
fn submit(
    services: &crate::gui_services::Services,
    path: PathBuf,
    generation: u64,
) -> Result<CancellationToken, async_service::Failure> {
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let service = services.clone();
    let mut context = OperationContext::new(
        SUBSYSTEM,
        path.to_string_lossy().into_owned(),
        Policy::ReplaceableRead,
    );
    context.generation = generation;
    services
        .handle()
        .submit(context, cancel.clone(), async move {
            let result = load(service.fs(), service.cpu(), path.clone(), &token)
                .await
                .map(Arc::new)
                .map_err(|error| format!("{error:#}"));
            Ok(vec![crate::Update::Table(path, generation, result)])
        })?;
    Ok(cancel)
}

/// One open table: the parsed file and its view state.
#[derive(Default)]
pub struct TableState {
    pub table: Option<Arc<Table>>,
    pub error: Option<String>,
    pub loading: bool,
    pub cancellation: Option<CancellationToken>,
    pub generation: u64,
    pub sort: Option<table::Sort>,
    pub filter: String,
    /// Visible row order, and the (sort, filter) it was built for.
    order: Option<(Option<table::Sort>, String, Vec<usize>)>,
}

impl Drop for TableState {
    fn drop(&mut self) {
        if let Some(cancellation) = &self.cancellation {
            cancellation.cancel();
        }
    }
}

impl TableState {
    /// Row indices in view order; rebuilt only when sort or filter changed.
    pub fn order(&mut self) -> &[usize] {
        let Some(table) = &self.table else {
            return &[];
        };
        let stale = self
            .order
            .as_ref()
            .is_none_or(|(sort, filter, _)| *sort != self.sort || *filter != self.filter);
        if stale {
            let rows = table::order(table, self.sort, &self.filter);
            self.order = Some((self.sort, self.filter.clone(), rows));
        }
        self.order.as_ref().map_or(&[], |(_, _, rows)| rows)
    }

    /// Next sort for a click on `column`: ascending, descending, then none.
    pub fn toggle_sort(&mut self, column: usize) {
        self.sort = match self.sort {
            Some(sort) if sort.column == column && sort.ascending => Some(table::Sort {
                column,
                ascending: false,
            }),
            Some(sort) if sort.column == column => None,
            _ => Some(table::Sort {
                column,
                ascending: true,
            }),
        };
    }
}

/// Open tables by path. Like image previews, a table no pane painted this
/// frame is dropped (and its load cancelled).
#[derive(Default)]
pub struct Tables {
    pub entries: HashMap<PathBuf, TableState>,
    visible: HashSet<PathBuf>,
    generation: u64,
}

impl Tables {
    pub fn begin_frame(&mut self) {
        self.visible.clear();
    }

    pub fn mark_visible(&mut self, path: &Path) {
        self.visible.insert(path.into());
    }

    pub fn prune(&mut self) {
        let visible = &self.visible;
        self.entries.retain(|path, _| visible.contains(path));
    }

    /// Forget `path` so the next paint reads it again.
    pub fn invalidate(&mut self, path: &Path) {
        self.entries.remove(path);
    }

    /// The state of `path`, starting its load when needed; `None` when too
    /// many tables are open.
    pub fn state(
        &mut self,
        services: &crate::gui_services::Services,
        path: &Path,
        reload: bool,
    ) -> Option<&mut TableState> {
        if !self.entries.contains_key(path) && self.entries.len() >= MAX_OPEN {
            return None;
        }
        let state = self.entries.entry(path.into()).or_default();
        if reload {
            if let Some(cancel) = state.cancellation.take() {
                cancel.cancel();
            }
            state.loading = false;
            state.error = None;
        }
        if reload || (!state.loading && state.table.is_none() && state.error.is_none()) {
            self.generation = self.generation.wrapping_add(1);
            state.generation = self.generation;
            state.cancellation = submit(services, path.into(), state.generation).ok();
            state.loading = state.cancellation.is_some();
        }
        Some(state)
    }

    pub fn loaded(&mut self, path: &Path, generation: u64, result: Result<Arc<Table>, String>) {
        let Some(state) = self
            .entries
            .get_mut(path)
            .filter(|state| state.generation == generation)
        else {
            return;
        };
        state.loading = false;
        state.cancellation = None;
        match result {
            Ok(table) => {
                state.table = Some(table);
                state.order = None;
                state.error = None;
            }
            Err(error) => state.error = Some(error),
        }
    }

    pub fn cancelled(&mut self, generation: u64) {
        if let Some(state) = self
            .entries
            .values_mut()
            .find(|state| state.generation == generation)
        {
            state.loading = false;
            state.cancellation = None;
        }
    }
}
