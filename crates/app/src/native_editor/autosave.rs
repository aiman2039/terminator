//! Autosave (Settings → Editor): on focus loss, or after a pause in
//! typing. Never while the file changed on disk, after a failed save,
//! or for read-only buffers.
use super::*;
use terminator_core::editor_prefs::Autosave;

/// One view's facts the autosave decision needs.
pub(super) struct AutosaveInput<'a> {
    pub(super) path: &'a Path,
    pub(super) source_id: &'a str,
    pub(super) focused: bool,
    pub(super) read_only: bool,
}

impl NativeDoc {
    /// Whether an automatic save may run now: unsaved edits, nothing in
    /// flight, and no disk change or earlier failure to resolve first.
    pub(super) fn autosave_allowed(&self) -> bool {
        let on_disk = self.last_known_mtime();
        self.dirty()
            && !self.saving
            && !self.loading
            && !self.force_save
            && self.save_error.is_none()
            && self
                .disk
                .reload_seen
                .is_none_or(|seen| seen == super::disk::Stamp(self.loaded_mtime))
            && on_disk == self.loaded_mtime
    }

    /// Track focus and typing for `mode`; true when a save is due now.
    /// `wait` receives how long until a delayed save is due.
    pub(super) fn autosave_due(
        &mut self,
        (mode, delay): (Autosave, Duration),
        (source_id, focused): (&str, bool),
        wait: &mut Option<Duration>,
    ) -> bool {
        let lost_focus = if focused {
            if self.extras.focus.as_deref() != Some(source_id) {
                self.extras.focus = Some(source_id.to_owned());
            }
            false
        } else if self.extras.focus.as_deref() == Some(source_id) {
            self.extras.focus = None;
            true
        } else {
            false
        };
        let tracked = self.last_edit_at().is_some();
        match mode {
            Autosave::Off => false,
            Autosave::FocusLoss => tracked && lost_focus && self.autosave_allowed(),
            Autosave::Delay => self.delay_autosave_due(delay, wait),
        }
    }

    /// When the buffer's current revision was first seen: its last edit.
    fn last_edit_at(&mut self) -> Option<Instant> {
        let revision = self.doc.as_ref().map(Buffer::revision)?;
        Some(match self.extras.last_edit {
            Some((seen, at)) if seen == revision => at,
            _ => {
                let now = Instant::now();
                self.extras.last_edit = Some((revision, now));
                now
            }
        })
    }

    /// Delayed autosave: true once `delay` passed since the last edit and
    /// a save is allowed. `wait` receives the time left. It needs no view,
    /// so a document whose view is hidden is saved too (#332).
    pub(super) fn delay_autosave_due(
        &mut self,
        delay: Duration,
        wait: &mut Option<Duration>,
    ) -> bool {
        let Some(edited) = self.last_edit_at() else {
            return false;
        };
        if !self.autosave_allowed() {
            return false;
        }
        let elapsed = edited.elapsed();
        *wait = (elapsed < delay).then(|| delay.saturating_sub(elapsed));
        wait.is_none()
    }
}

/// How often the app loop checks an autosave write for its end.
const SAVE_POLL: Duration = Duration::from_millis(100);

/// The earlier of two optional waits.
fn earliest(a: Option<Duration>, b: Option<Duration>) -> Option<Duration> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

impl App {
    /// Delayed autosave for every open document, drawn or not (#332). Runs
    /// in the app loop, so a file whose view is hidden (another tab, a
    /// minimized window) is still saved after the pause. Read-only buffers
    /// (rename applying, remote snapshot) and files with a pending close
    /// prompt are skipped. A view that draws later in the frame sees
    /// `saving` and does not save again. Writes run on the file pool; this
    /// only polls their answers.
    pub(crate) fn autosave_native_docs(&mut self, ctx: &egui::Context) {
        let prefs = appearance::editor_style().prefs;
        if prefs.editor_autosave != Autosave::Delay {
            return;
        }
        let delay = Duration::from_millis(u64::from(prefs.editor_autosave_delay_ms));
        let services = self.services.clone();
        let rename = self.native_rename.as_ref();
        let data = &self.paths.data;
        let prompts = &self.native_close_prompts;
        let mut next = None;
        let mut settled = Vec::new();
        for (path, doc) in &mut self.native_docs {
            if doc.saving {
                doc.poll();
                if doc.saving {
                    next = earliest(next, Some(SAVE_POLL));
                } else {
                    settled.push(path.clone());
                }
                continue;
            }
            let mut wait = None;
            let due = doc.delay_autosave_due(delay, &mut wait);
            if !due && wait.is_none() {
                continue;
            }
            let held = rename.is_some_and(|dialog| dialog.applying_to(path))
                || crate::remote_ui::is_snapshot(data, path)
                || prompts.iter().any(|request| request.path == *path);
            if held {
                continue;
            }
            if due {
                doc.start_save(&services);
                next = earliest(next, doc.saving.then_some(SAVE_POLL));
            }
            next = earliest(next, wait);
        }
        // A finished write reaches the language servers, as in the view.
        for path in &settled {
            self.native_lsp_saved(path);
        }
        if let Some(wait) = next {
            ctx.request_repaint_after(wait);
        }
    }
}

impl App {
    pub(super) fn native_autosave(&mut self, ctx: &egui::Context, input: AutosaveInput<'_>) {
        let AutosaveInput {
            path,
            source_id,
            focused,
            read_only,
        } = input;
        let prefs = appearance::editor_style().prefs;
        let delay = Duration::from_millis(u64::from(prefs.editor_autosave_delay_ms));
        let services = self.services.clone();
        let Some(doc) = self.native_docs.get_mut(path) else {
            return;
        };
        let mut wait = None;
        let due = doc.autosave_due(
            (prefs.editor_autosave, delay),
            (source_id, focused),
            &mut wait,
        );
        if due && !read_only {
            doc.start_save(&services);
        } else if let Some(wait) = wait {
            ctx.request_repaint_after(wait);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autosave_waits_for_the_delay_and_never_runs_on_a_disk_change() {
        let mut doc = NativeDoc::loaded(PathBuf::from("/tmp/autosave.txt"), "one");
        let delay = (Autosave::Delay, Duration::from_millis(0));
        let mut wait = None;
        assert!(!doc.autosave_due(delay, ("v", true), &mut wait), "clean");
        doc.doc.as_mut().unwrap().insert(3, " two");
        assert!(doc.autosave_due(delay, ("v", true), &mut wait));
        let slow = (Autosave::Delay, Duration::from_mins(1));
        doc.doc.as_mut().unwrap().insert(0, "x");
        assert!(!doc.autosave_due(slow, ("v", true), &mut wait));
        assert!(wait.is_some());
        // The file changed on disk: no automatic overwrite.
        doc.mtime_check = Some((Instant::now(), Some(std::time::SystemTime::now())));
        assert!(!doc.autosave_due(delay, ("v", true), &mut wait));
    }

    #[test]
    fn autosave_on_focus_loss_saves_only_when_the_view_loses_focus() {
        let mut doc = NativeDoc::loaded(PathBuf::from("/tmp/focus.txt"), "one");
        doc.doc.as_mut().unwrap().insert(3, "!");
        let mode = (Autosave::FocusLoss, Duration::ZERO);
        let mut wait = None;
        assert!(!doc.autosave_due(mode, ("v", true), &mut wait));
        assert!(!doc.autosave_due(mode, ("other", false), &mut wait));
        assert!(doc.autosave_due(mode, ("v", false), &mut wait));
        assert!(!doc.autosave_due(mode, ("v", false), &mut wait));
        assert!(!doc.autosave_due((Autosave::Off, Duration::ZERO), ("v", true), &mut wait));
    }

    #[test]
    fn hidden_dirty_document_becomes_due_after_the_delay_without_a_view() {
        let mut doc = NativeDoc::loaded(PathBuf::from("/tmp/hidden.txt"), "one");
        let mut wait = None;
        assert!(!doc.delay_autosave_due(Duration::ZERO, &mut wait), "clean");
        doc.doc.as_mut().unwrap().insert(3, " two");
        assert!(!doc.delay_autosave_due(Duration::from_mins(1), &mut wait));
        assert!(wait.is_some_and(|left| left <= Duration::from_mins(1)));
        assert!(doc.delay_autosave_due(Duration::ZERO, &mut wait));
        assert!(wait.is_none());
        // No view took part: focus tracking is untouched.
        assert!(doc.extras.focus.is_none());
    }

    #[test]
    fn hidden_document_changed_on_disk_or_saving_is_not_autosaved() {
        let mut doc = NativeDoc::loaded(PathBuf::from("/tmp/hidden-disk.txt"), "one");
        doc.doc.as_mut().unwrap().insert(3, "!");
        let mut wait = None;
        doc.saving = true;
        assert!(!doc.delay_autosave_due(Duration::ZERO, &mut wait), "saving");
        doc.saving = false;
        doc.mtime_check = Some((Instant::now(), Some(std::time::SystemTime::now())));
        assert!(!doc.delay_autosave_due(Duration::ZERO, &mut wait), "disk");
        assert!(wait.is_none());
    }

    #[test]
    fn earliest_wait_ignores_a_missing_one() {
        let short = Some(Duration::from_millis(5));
        let long = Some(Duration::from_secs(5));
        assert_eq!(earliest(None, None), None);
        assert_eq!(earliest(None, long), long);
        assert_eq!(earliest(long, short), short);
    }
}
