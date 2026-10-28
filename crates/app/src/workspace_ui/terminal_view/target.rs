use super::super::super::*;
use super::super::viewer::Viewer;
use super::TerminalFrame;
use egui_term::TerminalBackend;

/// The link target under the pointer this frame, and its resolve key.
pub(super) struct HoverTarget {
    target: Option<egui_term::LinkTarget>,
    pub(super) token: String,
    key: String,
    mouse_reporting: bool,
}

impl Viewer<'_> {
    /// Hit-test the pointer against the terminal's link targets. None when
    /// the backend is gone.
    pub(super) fn terminal_hover_target(
        &self,
        ui: &egui::Ui,
        frame: &TerminalFrame<'_>,
    ) -> Option<HoverTarget> {
        let session = frame.session;
        let sid = &session.id;
        let response = &frame.response;
        let dragging = ui.input(|i| i.pointer.any_down());
        let backend = self.app.backends.get(sid)?;
        #[cfg(feature = "test-support")]
        Self::record_scroll_evidence(ui, sid, backend, frame.focused);
        let mouse_reporting = backend
            .last_content()
            .terminal_mode
            .intersects(egui_term::TerminalMode::MOUSE_MODE);
        let target = if !frame.input_enabled || dragging || self.app.hover_popup.is_some() {
            None
        } else {
            response.hover_pos().and_then(|pos| {
                backend.target_at(pos.x - response.rect.left(), pos.y - response.rect.top())
            })
        };
        let token = target.as_ref().map(|t| t.text.clone()).unwrap_or_default();
        let key = format!("target:{}:{}:{}", sid, session.cwd.display(), token);
        Some(HoverTarget {
            target,
            token,
            key,
            mouse_reporting,
        })
    }

    #[cfg(feature = "test-support")]
    fn record_scroll_evidence(
        ui: &egui::Ui,
        sid: &String,
        backend: &TerminalBackend,
        focused: bool,
    ) {
        if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_none() {
            return;
        }
        let content = backend.last_content();
        let snapshot = (
            focused,
            content.display_offset,
            content.terminal_mode.bits(),
        );
        let key = egui::Id::unique(("scroll-evidence", sid));
        if ui.ctx().data(|d| d.get_temp::<(bool, usize, u32)>(key)) != Some(snapshot) {
            crate::console::Console::err(format_args!(
                "Scroll evidence: session={sid} focused={} offset={} modes={}",
                snapshot.0, snapshot.1, snapshot.2
            ));
            ui.ctx().data_mut(|d| d.insert_temp(key, snapshot));
        }
    }

    /// Resolve the hovered token to a file in the background, once.
    pub(super) fn request_hover_target(&mut self, frame: &TerminalFrame<'_>, hover: &HoverTarget) {
        let HoverTarget { token, key, .. } = hover;
        if !token.is_empty()
            && !self.app.targets.contains_key(key)
            && self.app.loading.insert(key.clone())
        {
            let _ = self.app.jobs.send(Job::ResolveTarget(
                key.clone(),
                token.clone(),
                frame.session.cwd.clone(),
            ));
        }
    }

    /// Underline a resolved hover target, remember it for copy, and open
    /// its file menu on click or the link modifier.
    pub(super) fn show_resolved_target(
        &mut self,
        ui: &mut egui::Ui,
        frame: &TerminalFrame<'_>,
        hover: &HoverTarget,
    ) {
        let sid = &frame.session.id;
        let response = &frame.response;
        let resolved = self.app.targets.get(&hover.key).cloned().flatten();
        let (Some(target), Some(resolved)) = (&hover.target, &resolved) else {
            return;
        };
        if ui.input(|i| i.pointer.any_down()) || hover.mouse_reporting {
            return;
        }
        self.app
            .remember_hover_copy(crate::workspace_ui::hover::RememberHoverCopyInput {
                ui,
                sid,
                target: resolved,
                rects: target
                    .rects
                    .iter()
                    .map(|rect| rect.translate(response.rect.min.to_vec2()))
                    .collect(),
                strip: self.strip,
            });
        for rect in &target.rects {
            let rect = rect.translate(response.rect.min.to_vec2());
            ui.painter().with_clip_rect(response.rect).line_segment(
                [rect.left_bottom(), rect.right_bottom()],
                egui::Stroke::new(1.0, appearance::color(&self.app.theme.accent)),
            );
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        if self.app.hover_popup.is_none()
            && !self.app.text_input_focused(ui.ctx())
            && (response.clicked() || frame.modifier_pressed)
        {
            let rect = target
                .rects
                .first()
                .copied()
                .unwrap_or(egui::Rect::ZERO)
                .translate(response.rect.min.to_vec2());
            self.app.hover_popup = Some(HoverPopup {
                session: sid.clone(),
                key: hover.key.clone(),
                target: resolved.clone(),
                rect,
            });
            #[cfg(feature = "test-support")]
            crate::console::Console::err(format_args!(
                "Terminal file menu opened: {}",
                resolved.display()
            ));
        }
    }

    /// A right click resolves the selection (or the hovered token) for the
    /// context menu's file actions.
    pub(super) fn terminal_secondary_click(
        &mut self,
        ui: &egui::Ui,
        frame: &TerminalFrame<'_>,
        token: &str,
    ) {
        if !frame.response.secondary_clicked() {
            return;
        }
        let session = frame.session;
        let sid = &session.id;
        let selected = self
            .app
            .backends
            .get(sid)
            .map_or_else(String::new, TerminalBackend::selectable_content);
        let text = if selected.trim().is_empty() {
            token.to_owned()
        } else {
            selected
        };
        let key = format!("target:{}:{}:{}", sid, session.cwd.display(), text);
        ui.ctx()
            .data_mut(|d| d.insert_temp(frame.menu_key, key.clone()));
        if !self.app.targets.contains_key(&key) && self.app.loading.insert(key.clone()) {
            let _ = self
                .app
                .jobs
                .send(Job::ResolveTarget(key, text, session.cwd.clone()));
        }
    }
}
