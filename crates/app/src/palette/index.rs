use super::*;

impl App {
    /// Watched directory roots driving the file index, sorted.
    fn file_index_roots(&self) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = self
            .visible_projects()
            .iter()
            .map(|project| project.path.clone())
            .collect();
        if roots.is_empty() {
            roots.extend(self.dirs.keys().cloned());
        }
        roots.sort();
        roots.dedup();
        roots
    }

    /// Kick a background index rescan when roots changed or the index is
    /// older than 30s; at most one scan flies at a time.
    pub(crate) fn refresh_file_index(&mut self) {
        if self.file_index_rx.is_some() {
            return;
        }
        let roots = self.file_index_roots();
        if roots != self.file_index_roots {
            self.file_index = terminator_native_edit::finder::FileIndex::empty();
            self.file_index_at = None;
        }
        let stale = self
            .file_index_at
            .is_none_or(|at| at.elapsed().as_secs() >= 30);
        if roots == self.file_index_roots && !stale {
            return;
        }
        self.file_index_roots.clone_from(&roots);
        self.file_index_rx = Some(spawn_scan(roots, MAX_INDEX_FILES));
    }

    /// Adopt a finished background scan (non-blocking).
    pub(crate) fn poll_file_index(&mut self) {
        if let Some(rx) = self.file_index_rx.as_ref() {
            match rx.try_recv() {
                Ok(index) => {
                    self.file_index = index;
                    self.file_index_at = Some(std::time::Instant::now());
                    self.file_index_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.file_index_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
    }

    pub(super) fn palette_project_root(&self) -> Option<&Path> {
        self.selected_project()
            .map(|project| project.path.as_path())
    }

    /// Other projects whose path sits inside the selected project.
    pub(super) fn palette_exclude_roots(&self) -> Vec<PathBuf> {
        let Some(selected) = self.selected_project() else {
            return Vec::new();
        };
        self.state
            .projects
            .iter()
            .filter(|project| {
                project.id != selected.id
                    && self.project_available(&project.id)
                    && project.path.starts_with(&selected.path)
            })
            .map(|project| project.path.clone())
            .collect()
    }

    /// The file belongs to the selected project, not to a nested one.
    pub(super) fn path_in_palette_project(&self, path: &Path) -> bool {
        let Some(selected) = self.selected_project() else {
            return false;
        };
        self.project_for_palette_path(path)
            .is_some_and(|owner| owner.id == selected.id)
    }

    /// No chip: every kind in the current project, except the project list.
    /// The Projects chip is the way to open another project.
    pub(super) fn palette_wants(&self, kind: u8) -> bool {
        if self.palette.filters == 0 {
            kind != FILTER_PROJECTS
        } else {
            self.palette.filters & kind != 0
        }
    }

    fn session_in_palette_project(&self, id: &str) -> bool {
        let Some(project) = self.selected_project() else {
            return false;
        };
        self.state
            .sessions
            .iter()
            .any(|session| session.id == id && session.project_id == project.id)
    }

    pub(super) fn palette_accept(&self, item: &PaletteItem) -> bool {
        let kind = item.filter_bit();
        if !self.palette_wants(kind) {
            return false;
        }
        // An empty field shows files, sessions, and agents. Actions,
        // settings, and symbols appear after the user types, or when
        // that filter is on.
        if self.palette.query.trim().is_empty()
            && self.palette.filters == 0
            && matches!(kind, FILTER_ACTIONS | FILTER_SETTINGS | FILTER_SYMBOLS)
        {
            return false;
        }
        match item {
            PaletteItem::File(path) | PaletteItem::Symbol { path, .. } => {
                self.path_in_palette_project(path)
            }
            PaletteItem::Session(id, _) | PaletteItem::Agent(id, _, _) => {
                self.session_in_palette_project(id)
            }
            _ => true,
        }
    }
}
