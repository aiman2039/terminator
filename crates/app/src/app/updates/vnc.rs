//! VNC connection, frame, and file transfer results.

use crate::*;
use terminator_core::vnc::{VncSession, VncTransferStatus};

impl App {
    pub(super) fn handle_vnc_transfer(
        &mut self,
        session: &str,
        result: Result<Option<VncTransferStatus>, String>,
    ) {
        match result {
            Ok(Some(status)) => {
                if let Some(view) = self.vnc_views.get_mut(session) {
                    view.transfer_status = Some(status);
                }
            }
            Ok(None) => {}
            Err(error) => {
                if let Some(view) = self.vnc_views.get_mut(session)
                    && let Some(status) = &mut view.transfer_status
                {
                    status.done = true;
                    status.error = Some(error.clone());
                }
                self.error = Some(error);
            }
        }
    }

    pub(super) fn handle_vnc_connected(
        &mut self,
        ctx: &egui::Context,
        session: VncSession,
        after: After,
    ) {
        let project = if !session.project_id.is_empty() {
            session.project_id.clone()
        } else if let Some(selected) = &self.selected {
            selected.clone()
        } else {
            return;
        };
        let tab = Tab::Vnc {
            id: id(),
            session: session.id,
            profile_id: session.profile_id,
            host: session.host,
            port: session.port,
            label: session.label,
        };
        self.place_gui_tab(project, tab, after);
        ctx.request_repaint();
    }

    pub(super) fn handle_vnc_tab_connected(
        &mut self,
        ctx: &egui::Context,
        tab_id: String,
        result: Result<VncSession, String>,
    ) {
        match result {
            Ok(session) => {
                if let Some(tab) = &mut self.vnc_fullscreen {
                    Self::retarget_vnc_tab(tab, &tab_id, &session);
                }
                for workspace in self.layouts.values_mut() {
                    for group in &mut workspace.tabs {
                        for tab in group
                            .primary
                            .iter_mut()
                            .chain(group.layout.iter_all_tabs_mut().map(|(_, t)| t))
                        {
                            Self::retarget_vnc_tab(tab, &tab_id, &session);
                        }
                    }
                }
                self.pending_layout_save = true;
            }
            Err(err) => {
                let view = self.vnc_views.entry(format!("draft:{tab_id}")).or_default();
                view.error = Some(err);
            }
        }
        ctx.request_repaint();
    }

    /// Points the VNC tab `tab_id` at its newly connected session.
    fn retarget_vnc_tab(tab: &mut Tab, tab_id: &str, session: &VncSession) {
        if let Tab::Vnc {
            id,
            session: sid,
            profile_id,
            host,
            port,
            label,
        } = tab
            && id.as_str() == tab_id
        {
            sid.clone_from(&session.id);
            profile_id.clone_from(&session.profile_id);
            host.clone_from(&session.host);
            *port = session.port;
            label.clone_from(&session.label);
        }
    }

    pub(super) fn handle_vnc_frame(
        &mut self,
        ctx: &egui::Context,
        session: &str,
        result: Result<Option<crate::vnc_ui::VncDecodedFrame>, String>,
    ) {
        match result {
            Ok(Some(decoded)) => {
                self.apply_vnc_frame(ctx, decoded);
            }
            Ok(None) => {
                if let Some(view) = self.vnc_views.get_mut(session) {
                    view.fetching = false;
                }
            }
            Err(err) => {
                if let Some(view) = self.vnc_views.get_mut(session) {
                    view.fetching = false;
                    view.error = Some(err);
                }
            }
        }
        ctx.request_repaint();
    }
}
