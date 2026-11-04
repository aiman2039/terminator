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
        let Some(revision) = self.doc.as_ref().map(Buffer::revision) else {
            return false;
        };
        let edited = match self.extras.last_edit {
            Some((seen, at)) if seen == revision => at,
            _ => {
                let now = Instant::now();
                self.extras.last_edit = Some((revision, now));
                now
            }
        };
        if !self.autosave_allowed() {
            return false;
        }
        match mode {
            Autosave::Off => false,
            Autosave::FocusLoss => lost_focus,
            Autosave::Delay => {
                let elapsed = edited.elapsed();
                *wait = (elapsed < delay).then(|| delay.saturating_sub(elapsed));
                wait.is_none()
            }
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
}
