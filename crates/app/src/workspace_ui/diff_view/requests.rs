use super::super::super::*;
use super::{DiffFrame, DiffPass};
use diff::actions::{Kind, Request};

impl App {
    /// Send this frame's save, discard, undo, or hunk request.
    pub(super) fn submit_diff_request(&mut self, frame: &DiffFrame<'_>, pass: &mut DiffPass) {
        let request = Self::diff_request(frame, pass);
        if !frame.read_only
            && (frame.enabled || pass.save_edits)
            && let Some(request) = request
        {
            let review = &mut pass.review;
            review.saving_edits = pass.save_edits;
            review.busy = true;
            review.confirm_discard = None;
            review.message = None;
            let _ = self
                .jobs
                .send(Job::DiffAction(frame.tab.clone(), Box::new(request)));
        }
    }

    fn diff_request(frame: &DiffFrame<'_>, pass: &mut DiffPass) -> Option<Request> {
        if pass.save_edits {
            return pass.review.buffer.as_ref().map(|edit| Request::Save {
                document: std::sync::Arc::clone(&edit.document),
                text: edit.text.clone(),
            });
        }
        if let Some(index) = pass.confirmed {
            return Self::hunk_request(frame, pass, (index, Kind::Discard));
        }
        if pass.undo {
            return pass.review.undo.last().cloned().map(Request::Undo);
        }
        let (index, kind) = pass.action?;
        pass.review.selected = Some(index);
        if kind == Kind::Discard {
            pass.review.confirm_discard = Some(index);
            return None;
        }
        Self::hunk_request(frame, pass, (index, kind))
    }

    fn hunk_request(
        frame: &DiffFrame<'_>,
        pass: &mut DiffPass,
        (index, kind): (usize, Kind),
    ) -> Option<Request> {
        let lines = pass.review.picked_lines(index);
        frame.doc.as_ref().map(|document| Request::Hunk {
            document: std::sync::Arc::clone(document),
            hunk: index,
            kind,
            lines,
        })
    }

    /// Expand a collapsed gap the controls asked to reveal.
    pub(super) fn expand_diff_gap(&mut self, frame: &DiffFrame<'_>, pass: &mut DiffPass) {
        if !pass.review.busy
            && let Some(gap) = pass.gap
            && let Some(doc) = &frame.doc
        {
            pass.review.confirm_discard = None;
            self.loading.insert(frame.key.clone());
            let (gap, part) = gap;
            let _ = self.jobs.send(Job::DiffExpand(
                frame.tab.clone(),
                std::sync::Arc::clone(doc),
                gap,
                part,
            ));
        }
    }

    /// Rebuild the view for a refresh or a new context size.
    pub(super) fn refresh_diff_view(&mut self, frame: &DiffFrame<'_>, pass: &mut DiffPass) {
        let review = &mut pass.review;
        if review.busy || !(pass.refresh || pass.context.is_some()) {
            return;
        }
        // `review` is checked out of the map while this pane renders.
        let mut view = self.diff_view_for(
            &frame.key,
            Some(pass.context.unwrap_or(review.context.max(3))),
        );
        view.whitespace = review.ignore_whitespace;
        if pass.collapse {
            view.revealed.clear();
        }
        review.jump = true;
        review.confirm_discard = None;
        self.loading.insert(frame.key.clone());
        let _ = self.jobs.send(Job::DiffContext(frame.tab.clone(), view));
    }
}
