//! Dirty-buffer close prompts and save-then-close requests, each tagged
//! with the flow that asked. Several flows (a pane's own close, quit, a
//! workspace close) can wait on native buffers at once; an answer or a
//! settled save resumes only its own flow, and a failed save stops it
//! instead of leaving it waiting.
use super::*;

/// The flow a native close prompt or save-then-close request belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CloseOwner {
    /// One view's own close: dock X, `:wq`, or the in-pane bar.
    Pane,
    /// Quit (window close, Quit, an update's relaunch).
    Quit,
    /// The close of one workspace tab: project and tab id.
    Workspace(String, String),
}

/// One prompt or save-then-close request for a file.
#[derive(Clone, Debug)]
pub(crate) struct NativeCloseRequest {
    pub(crate) path: PathBuf,
    /// The view that asked, when a view asked. Without one the close
    /// takes every docked copy (quit and workspace flows).
    pub(crate) issuer: Option<CloseIssuer>,
    pub(crate) owner: CloseOwner,
}

impl NativeCloseRequest {
    pub(crate) fn new(path: PathBuf, issuer: Option<CloseIssuer>, owner: CloseOwner) -> Self {
        Self {
            path,
            issuer,
            owner,
        }
    }

    /// Asked from a view: shown as a bar inside that view, not a modal.
    pub(crate) fn in_pane(&self) -> bool {
        self.owner == CloseOwner::Pane && self.issuer.is_some()
    }

    fn same(&self, other: &Self) -> bool {
        self.path == other.path && self.owner == other.owner
    }

    fn name(&self) -> String {
        self.path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    }
}

/// Where a save-then-close request stands.
enum AfterSave {
    Waiting,
    Saved,
    /// The save ended (failed, refused, or edits came in during it) and
    /// the buffer still has unsaved changes.
    Failed,
}

impl App {
    /// Ask about one dirty buffer for `owner`. A newer ask from the same
    /// owner for the same file replaces the older one; asks from other
    /// owners keep their places.
    pub(crate) fn ask_native_close(&mut self, request: NativeCloseRequest) {
        self.native_close_prompts.retain(|p| !p.same(&request));
        self.native_close_prompts.push(request);
    }

    /// Whether `owner` still waits on a prompt or a save.
    pub(crate) fn native_close_waits_for(&self, owner: &CloseOwner) -> bool {
        self.native_close_prompts
            .iter()
            .chain(&self.native_close_after_save)
            .any(|request| request.owner == *owner)
    }

    /// The in-pane prompt for `path`, if a view asked about it.
    pub(super) fn in_pane_close_prompt(&self, path: &Path) -> Option<NativeCloseRequest> {
        self.native_close_prompts
            .iter()
            .find(|request| request.in_pane() && request.path == path)
            .cloned()
    }

    /// Drop one prompt without acting on it.
    pub(super) fn forget_close_prompt(&mut self, request: &NativeCloseRequest) {
        self.native_close_prompts.retain(|p| !p.same(request));
    }

    /// Apply a prompt answer: `None` is Cancel, which stops the owner's
    /// flow (cancels the quit, stops the workspace close).
    pub(crate) fn answer_native_close(
        &mut self,
        request: NativeCloseRequest,
        choice: Option<NativeCloseChoice>,
    ) {
        self.forget_close_prompt(&request);
        match choice {
            Some(choice) => self.apply_native_close_choice(request, choice),
            None => self.stop_native_close_owner(&request, "still has unsaved changes"),
        }
    }

    /// Discard closes the issuing view when the prompt names one (scoped
    /// like `:q`, never every copy) and every copy for flow prompts.
    /// Save-and-close defers the issuing pane through the save so the
    /// settled close lands on the view that asked.
    pub(crate) fn apply_native_close_choice(
        &mut self,
        request: NativeCloseRequest,
        choice: NativeCloseChoice,
    ) {
        match choice {
            NativeCloseChoice::Discard => {
                let owner = request.owner.clone();
                match request.issuer {
                    Some(issuer) => self.close_native_tab(&request.path, false, Some(issuer)),
                    None => self.close_native_tab(&request.path, true, None),
                }
                self.resume_native_close_owner(&owner);
            }
            NativeCloseChoice::SaveClose => {
                if let Some(doc) = self.native_docs.get_mut(&request.path) {
                    doc.force_save = false;
                    doc.start_save(&self.services.clone());
                }
                self.native_close_after_save.retain(|r| !r.same(&request));
                self.native_close_after_save.push(NativeCloseRequest {
                    issuer: request.issuer.map(CloseIssuer::deferred),
                    ..request
                });
            }
        }
    }

    fn after_save_state(&self, path: &Path) -> AfterSave {
        match self.native_docs.get(path) {
            Some(doc) if doc.saving || doc.loading => AfterSave::Waiting,
            Some(doc) if doc.dirty() => AfterSave::Failed,
            _ => AfterSave::Saved,
        }
    }

    /// The view-side check for this view's own `:wq` / Save and close:
    /// queue the deferred close once the save settled. Returns true when
    /// the view should stop rendering. Other owners and failed saves are
    /// handled by [`App::settle_native_closes`].
    pub(super) fn native_close_after_save_settled(&mut self, path: &Path) -> bool {
        let Some(index) = self
            .native_close_after_save
            .iter()
            .position(|r| r.path == path && r.owner == CloseOwner::Pane)
        else {
            return false;
        };
        if !matches!(self.after_save_state(path), AfterSave::Saved) {
            return false;
        }
        // Deferred: the workspace is checked out of `layouts` while this
        // view renders. Consumed here so every other copy rendering later
        // does not queue its own close for the same request.
        let request = self.native_close_after_save.remove(index);
        self.pending_native_close
            .push((request.path, false, request.issuer));
        true
    }

    /// Settle every save-then-close request, also for files no view
    /// renders. Runs outside rendering, so closes apply directly. A
    /// finished save closes and resumes its owner; a save that ended
    /// with unsaved changes left stops its owner instead of waiting.
    pub(crate) fn settle_native_closes(&mut self) {
        if self.native_close_after_save.is_empty() {
            return;
        }
        let mut paths: Vec<PathBuf> = self
            .native_close_after_save
            .iter()
            .map(|request| request.path.clone())
            .collect();
        paths.sort();
        paths.dedup();
        for path in &paths {
            if let Some(doc) = self.native_docs.get_mut(path) {
                doc.poll();
            }
            // A write that just landed reaches the servers before the close.
            self.native_lsp_saved(path);
        }
        for request in std::mem::take(&mut self.native_close_after_save) {
            match self.after_save_state(&request.path) {
                AfterSave::Waiting => self.native_close_after_save.push(request),
                AfterSave::Saved => {
                    self.close_native_tab(&request.path, false, request.issuer);
                    self.resume_native_close_owner(&request.owner);
                }
                AfterSave::Failed => {
                    if let Some(doc) = self.native_docs.get_mut(&request.path)
                        && doc.save_error.is_none()
                    {
                        doc.set_notice("Not closed: the file still has unsaved changes");
                    }
                    self.stop_native_close_owner(&request, "was not saved");
                }
            }
        }
    }

    /// A prompt or save for `owner` resolved: continue that flow.
    fn resume_native_close_owner(&mut self, owner: &CloseOwner) {
        // Quit re-checks every frame (`continue_app_quit`); a pane close
        // is already done. A workspace close that is the current one
        // continues in `poll_workspace_close`; a late one (its sessions
        // already closed) finishes here.
        if let CloseOwner::Workspace(project, tab) = owner
            && self.close_workspace.as_ref() != Some(&(project.clone(), tab.clone()))
            && !self.native_close_waits_for(owner)
        {
            self.close_workspace_tab_now(project, tab);
        }
    }

    /// Stop the flow that owns `request`; its other prompts go with it.
    fn stop_native_close_owner(&mut self, request: &NativeCloseRequest, why: &str) {
        let name = request.name();
        match &request.owner {
            CloseOwner::Pane => {}
            CloseOwner::Quit => {
                if self.pending_app_quit {
                    self.info = Some(format!("Quit stopped: {name} {why}."));
                }
                self.cancel_app_quit();
            }
            CloseOwner::Workspace(project, tab) => {
                let owner = request.owner.clone();
                self.native_close_prompts.retain(|p| p.owner != owner);
                self.native_close_after_save.retain(|p| p.owner != owner);
                if self.close_workspace.as_ref() == Some(&(project.clone(), tab.clone())) {
                    self.abort_workspace_close();
                }
                self.info = Some(format!("Stopped closing tabs: {name} {why}."));
            }
        }
    }

    /// Modal for flow prompts (quit, workspace close, a dock close with no
    /// issuing view). One at a time, oldest first; in-pane prompts render
    /// inside their views instead.
    pub(crate) fn native_close_modal(&mut self, ctx: &egui::Context) {
        let Some(request) = self
            .native_close_prompts
            .iter()
            .find(|request| !request.in_pane())
            .cloned()
        else {
            return;
        };
        if !self.native_dirty(&request.path) {
            // Saved or reloaded meanwhile: nothing left to ask.
            self.forget_close_prompt(&request);
            if request.owner == CloseOwner::Pane {
                self.close_native_tab(&request.path, false, request.issuer.clone());
            }
            self.resume_native_close_owner(&request.owner);
            return;
        }
        let window = self.popups.window(ctx, "Unsaved changes");
        let dialog =
            crate::appearance::Confirm::new(format!("{} has unsaved changes.", request.name()), "")
                .destructive(crate::appearance::ConfirmButton::new("Discard changes"))
                .default_action(crate::appearance::ConfirmButton::new("Save and close"));
        // Closing the dialog is its Cancel. Sparkle keeps waiting if a quit
        // prompt is only hidden.
        let choice =
            crate::appearance::confirm(ctx, window, dialog, |_| {}).map(|answer| match answer {
                crate::appearance::ConfirmAnswer::Default => Some(NativeCloseChoice::SaveClose),
                crate::appearance::ConfirmAnswer::Destructive => Some(NativeCloseChoice::Discard),
                crate::appearance::ConfirmAnswer::Cancel => None,
            });
        if let Some(choice) = choice {
            self.answer_native_close(request, choice);
        }
    }
}
