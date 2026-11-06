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

    /// Whether `path` (a file, or any file under a folder) has an error,
    /// for the Explorer's red names.
    #[must_use]
    pub fn has_errors_under(&self, path: &Path) -> bool {
        self.diagnostics.iter().any(|(file, items)| {
            file.starts_with(path)
                && items
                    .iter()
                    .any(|item| item.severity == Some(L::Severity::Error))
        })
    }

    /// All diagnostics for one path, for the problems panel.
    pub fn problems_for(&self, path: &Path) -> Vec<L::Diagnostic> {
        self.diagnostics.get(path).cloned().unwrap_or_default()
    }

    /// Changes whenever any file's diagnostics change.
    #[must_use]
    pub fn diagnostics_generation(&self) -> u64 {
        self.diagnostics_generation
    }

    pub(super) fn bump_diagnostics(&mut self) {
        self.diagnostics_generation = self.diagnostics_generation.wrapping_add(1);
    }

    /// Language diagnostics plus spelling findings, for the editor's
    /// underlines and hover.
    pub fn editor_diagnostics(&self, path: &Path) -> Vec<L::Diagnostic> {
        let mut items = self.problems_for(path);
        if let Some(spelling) = self.spell_diagnostics.get(path) {
            items.extend(spelling.iter().cloned());
        }
        items
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
    /// status bar. Missing severity counts toward the total only. Counted
    /// once per diagnostics generation: the status bar asks every frame.
    #[must_use]
    pub fn problem_counts(&self) -> (usize, usize, usize) {
        if let Some((generation, counts)) = self.problem_totals.get()
            && generation == self.diagnostics_generation
        {
            return counts;
        }
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
        let counts = (errors, warnings, total);
        self.problem_totals
            .set(Some((self.diagnostics_generation, counts)));
        counts
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
        if let Some(note) = self.trust_note(&doc.server.root) {
            return Some(note);
        }
        let server = self.servers.get(&doc.server)?;
        if server.ready {
            return None;
        }
        Some(format!("Starting {}…", doc.server.language))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn problem_counts_are_counted_again_only_when_diagnostics_change() {
        let mut manager = LspManager::trusting();
        let error = |line| L::Diagnostic {
            range: L::Range {
                start: L::Position { line, character: 0 },
                end: L::Position { line, character: 1 },
            },
            severity: Some(L::Severity::Error),
            message: "bad".into(),
        };
        manager
            .diagnostics
            .insert(crate::test_path("/tmp/a.rs"), vec![error(0)]);
        assert_eq!(manager.problem_counts(), (1, 0, 1));
        manager
            .diagnostics
            .insert(crate::test_path("/tmp/b.rs"), vec![error(1)]);
        assert_eq!(manager.problem_counts(), (1, 0, 1), "same generation");
        manager.bump_diagnostics();
        assert_eq!(manager.problem_counts(), (2, 0, 2));
    }
}
