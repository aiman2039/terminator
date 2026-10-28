use super::*;

impl LspManager {
    /// Worst severity per buffer row, sorted for the view's binary
    /// search. Lines beyond `u32` never occur; they are skipped.
    pub fn row_marks(&self, path: &Path) -> Vec<DiagnosticMark> {
        let mut worst: HashMap<usize, L::Severity> = HashMap::new();
        if let Some(items) = self.diagnostics.get(path) {
            for item in items {
                let Ok(line) = usize::try_from(item.range.start.line) else {
                    continue;
                };
                worst
                    .entry(line)
                    .and_modify(|known| {
                        if let Some(severity) = item.severity
                            && severity < *known
                        {
                            *known = severity;
                        }
                    })
                    .or_insert_with(|| item.severity.unwrap_or(L::Severity::Hint));
            }
        }
        let mut marks: Vec<DiagnosticMark> = worst
            .into_iter()
            .map(|(line, severity)| DiagnosticMark { line, severity })
            .collect();
        marks.sort_by_key(|mark| mark.line);
        marks
    }

    /// All diagnostics for one path, for the problems panel.
    pub fn problems_for(&self, path: &Path) -> Vec<L::Diagnostic> {
        self.diagnostics.get(path).cloned().unwrap_or_default()
    }

    /// Every tracked file's diagnostics, sorted by path, for the
    /// global Problems panel.
    #[must_use]
    pub fn all_problems(&self) -> Vec<(PathBuf, Vec<L::Diagnostic>)> {
        let mut files: Vec<(PathBuf, Vec<L::Diagnostic>)> = self
            .diagnostics
            .iter()
            .filter(|(_, items)| !items.is_empty())
            .map(|(path, items)| (path.clone(), items.clone()))
            .collect();
        files.sort_by(|a, b| a.0.cmp(&b.0));
        files
    }

    /// `(errors, warnings, total)` across every tracked file, for the
    /// status bar. Missing severity counts toward the total only.
    #[must_use]
    pub fn problem_counts(&self) -> (usize, usize, usize) {
        let mut errors: usize = 0;
        let mut warnings: usize = 0;
        let mut total: usize = 0;
        for items in self.diagnostics.values() {
            for item in items {
                total = total.saturating_add(1);
                match item.severity {
                    Some(L::Severity::Error) => errors = errors.saturating_add(1),
                    Some(L::Severity::Warning) => warnings = warnings.saturating_add(1),
                    _ => {}
                }
            }
        }
        (errors, warnings, total)
    }

    /// Whether a document is tracked for `path` (a server was consulted).
    #[must_use]
    pub fn tracked(&self, path: &Path) -> bool {
        self.docs.contains_key(path)
    }

    /// Why `path` has no ready server, if tracked and not ready.
    #[must_use]
    pub fn server_note(&self, path: &Path) -> Option<String> {
        self.server_status(path)
    }

    /// Why `path` has no working server, if that is the case: missing
    /// binary, repeated crashes, or still starting. `None` means ready.
    pub(super) fn server_status(&self, path: &Path) -> Option<String> {
        let doc = self.docs.get(path)?;
        if let Some(note) = self.unavailable.get(doc.server.language) {
            return Some(note.clone());
        }
        let server = self.servers.get(&doc.server)?;
        if server.ready {
            return None;
        }
        Some(format!("Starting {}…", doc.server.language))
    }
}
