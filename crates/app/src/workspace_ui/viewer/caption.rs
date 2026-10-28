use super::super::super::*;
use super::super::tabs::snapshot_rows;
use super::Viewer;

/// Git, stack, vertical split, and horizontal split buttons.
type CaptionActions = (
    egui::Response,
    Option<egui::Response>,
    egui::Response,
    egui::Response,
);

/// Per-frame facts about one main-dock terminal caption.
struct CaptionFrame<'a> {
    session: &'a Session,
    editing: bool,
    /// A lone pane's name is already the workspace tab (or the OS window
    /// title when floated), so its caption paints no title.
    lone: bool,
    is_markdown: bool,
    pane: Option<egui_dock::NodePath>,
}

/// The painted caption row and its buttons.
struct Caption {
    response: egui::Response,
    close: Option<egui::Response>,
    actions: Option<CaptionActions>,
    clicks: CaptionClicks,
}

/// What the caption's buttons reported this frame.
#[derive(Clone, Copy)]
struct CaptionClicks {
    closing: bool,
    git: bool,
    stack: bool,
    split_vertical: bool,
    split_horizontal: bool,
    on_control: bool,
}

impl CaptionClicks {
    fn read(close: Option<&egui::Response>, actions: Option<&CaptionActions>) -> Self {
        Self {
            closing: close.is_some_and(eframe::egui::Response::clicked),
            git: actions.is_some_and(|(git, _, _, _)| git.clicked()),
            stack: actions.is_some_and(|(_, stack, _, _)| {
                stack.as_ref().is_some_and(eframe::egui::Response::clicked)
            }),
            split_vertical: actions.is_some_and(|(_, _, split, _)| split.clicked()),
            split_horizontal: actions.is_some_and(|(_, _, _, split)| split.clicked()),
            on_control: close.is_some_and(eframe::egui::Response::hovered)
                || actions.is_some_and(|(git, stack, vertical, horizontal)| {
                    git.hovered()
                        || stack.as_ref().is_some_and(eframe::egui::Response::hovered)
                        || vertical.hovered()
                        || horizontal.hovered()
                }),
        }
    }
}

impl Viewer<'_> {
    /// Main-dock caption above a terminal: title, Git, splits, and close;
    /// a drag handle for moving the pane; and the pane context menu.
    pub(super) fn pane_caption(&mut self, ui: &mut egui::Ui, session: &Session, editing: bool) {
        let sid = &session.id;
        let pane = self.pane_for(&Tab::Terminal(sid.clone()).key());
        ui.spacing_mut().item_spacing.y = 2.0;
        let frame = CaptionFrame {
            session,
            editing,
            lone: self.lone_pane(),
            is_markdown: session.remote.is_none() && markdown::available(session),
            pane,
        };
        let caption = self.paint_caption(ui, &frame);
        #[cfg(feature = "test-support")]
        Self::record_caption(ui, &frame, &caption);
        self.caption_rename(ui, &frame, &caption);
        #[cfg(feature = "test-support")]
        Self::record_caption_controls(ui, sid, &caption);
        self.caption_drag(ui, &frame, &caption);
        self.apply_caption_clicks(&frame, &caption);
        appearance::context_menu(&caption.response, |ui| {
            if let Some(pane) = frame.pane {
                self.context_menu(ui, &mut Tab::Terminal(sid.clone()), pane);
            }
        });
    }

    fn lone_pane(&self) -> bool {
        if self.window.is_some() {
            self.app
                .float_pane_tabs
                .values()
                .map(Vec::len)
                .sum::<usize>()
                == 1
        } else {
            self.app
                .pane_index
                .as_ref()
                .is_some_and(|index| index.tabs == 1)
        }
    }

    fn paint_caption(&mut self, ui: &mut egui::Ui, frame: &CaptionFrame<'_>) -> Caption {
        let (response, close, actions) = if frame.is_markdown {
            let header = self.markdown_header(ui, frame.session, frame.editing);
            (header.0, header.1, None)
        } else {
            let bar = self.terminal_caption_bar(ui, frame);
            (
                bar.bar,
                Some(bar.close),
                Some((bar.git, bar.stack, bar.split_vertical, bar.split_horizontal)),
            )
        };
        let clicks = CaptionClicks::read(close.as_ref(), actions.as_ref());
        Caption {
            response,
            close,
            actions,
            clicks,
        }
    }

    fn terminal_caption_bar(
        &self,
        ui: &mut egui::Ui,
        frame: &CaptionFrame<'_>,
    ) -> appearance::TerminalBar {
        let session = frame.session;
        let sid = &session.id;
        let branch = self.app.branch_at(&session.cwd);
        let git_tip = match &branch {
            Some(name) => format!("{name}\nOpen Git"),
            None => "Open Git".into(),
        };
        let vertical_tip = self.app.action_tip("Split vertically", Action::SplitRight);
        let horizontal_tip = self.app.action_tip("Split horizontally", Action::SplitDown);
        let leading = self.app.tab_leading(sid);
        appearance::terminal_bar(
            ui,
            appearance::TerminalBarSpec {
                title: if frame.editing || frame.lone {
                    ""
                } else {
                    &session.label
                },
                title_tint: Some(self.app.terminal_title_color(sid)),
                active: self.app.active_session.as_ref() == Some(sid),
                branch: branch.as_deref(),
                status: self.app.terminal_status_color(session),
                git_tip: &git_tip,
                // No leaf tab bar on a lone pane, so its `+` lives here
                // instead. Hidden without a leaf to avoid dead clicks.
                stack_tip: frame.pane.is_some().then_some("New tab in this split"),
                vertical_tip: &vertical_tip,
                horizontal_tip: &horizontal_tip,
                brand: leading.brand,
                status_icon: leading.status.map(|(icon, _, _)| icon),
                status_tint: leading.status.map(|(_, tint, _)| tint),
                spin: leading.status.is_some_and(|(_, _, spin)| spin),
                kind: if leading.status.is_none() {
                    leading.kind
                } else {
                    None
                },
            },
        )
    }

    #[cfg(feature = "test-support")]
    fn record_caption(ui: &egui::Ui, frame: &CaptionFrame<'_>, caption: &Caption) {
        if frame.is_markdown || frame.editing {
            return;
        }
        let label = &frame.session.label;
        // Recorded even for lone panes (whose title the workspace tab
        // already shows) so fixtures can address their caption menus.
        diagnostics::record(
            ui.ctx(),
            &format!("pane-caption:{label}"),
            caption.response.rect,
        );
        // The bar paints no title for lone panes (see the `title` argument
        // above), so only titled panes record one: fixtures can tell title
        // duplication.
        if !frame.lone {
            diagnostics::record(
                ui.ctx(),
                &format!("pane-title:{label}"),
                caption.response.rect,
            );
        }
    }

    fn caption_rename(&mut self, ui: &mut egui::Ui, frame: &CaptionFrame<'_>, caption: &Caption) {
        if !frame.editing {
            return;
        }
        let response = &caption.response;
        let controls_left = caption
            .actions
            .as_ref()
            .map(|(git, _, _, _)| git.rect.left() - 4.0)
            .unwrap_or_else(|| {
                response.rect.right()
                    - if caption.close.is_some() && !frame.is_markdown {
                        28.0
                    } else {
                        8.0
                    }
            });
        self.app
            .inline_rename(super::super::menus::InlineRenameInput {
                ui,
                sid: &frame.session.id,
                surface: RenameSurface::Pane,
                rect: egui::Rect::from_min_max(
                    egui::pos2(response.rect.min.x + 8.0, response.rect.min.y + 1.0),
                    egui::pos2(controls_left, response.rect.bottom() - 1.0),
                ),
            });
    }

    #[cfg(feature = "test-support")]
    fn record_caption_controls(ui: &egui::Ui, sid: &str, caption: &Caption) {
        if let Some(close) = &caption.close {
            diagnostics::record(ui.ctx(), &format!("editor-close:{sid}"), close.rect);
            diagnostics::record(ui.ctx(), &format!("pane-close:{sid}"), close.rect);
        }
        if let Some((git, stack, vertical, horizontal)) = &caption.actions {
            diagnostics::record(ui.ctx(), &format!("pane-git:{sid}"), git.rect);
            if let Some(stack) = stack {
                diagnostics::record(ui.ctx(), &format!("pane-stack:{sid}"), stack.rect);
            }
            diagnostics::record(
                ui.ctx(),
                &format!("pane-split-vertical:{sid}"),
                vertical.rect,
            );
            diagnostics::record(
                ui.ctx(),
                &format!("pane-split-horizontal:{sid}"),
                horizontal.rect,
            );
        }
        diagnostics::record(ui.ctx(), &format!("pane-drag:{sid}"), caption.response.rect);
    }

    /// Caption drag starts a pane move. The drop lands on another split
    /// leaf (rearrange) or a workspace strip tab (move across top-level
    /// tabs); clicks still focus as before. Floating windows opt out:
    /// cross-window drops are unsupported, so a drag starting here could
    /// never land.
    fn caption_drag(&mut self, ui: &egui::Ui, frame: &CaptionFrame<'_>, caption: &Caption) {
        let sid = &frame.session.id;
        let response = &caption.response;
        let clicks = caption.clicks;
        if response.drag_started()
            && self.window.is_none()
            && !frame.editing
            && !clicks.closing
            && !clicks.on_control
        {
            self.app.drag.pane = Some(Tab::Terminal(sid.clone()));
            // Snapshot once: the ghost reuses it every frame instead of
            // re-reading the live grid while it scrolls.
            self.app.drag.pane_snapshot = self
                .app
                .backends
                .get(sid)
                .map(snapshot_rows)
                .unwrap_or_default();
        }
        if self.app.drag.pane.as_ref() == Some(&Tab::Terminal(sid.clone()))
            && response.hovered()
            && ui.input(|i| i.pointer.any_down())
        {
            ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grabbing);
        }
    }

    fn apply_caption_clicks(&mut self, frame: &CaptionFrame<'_>, caption: &Caption) {
        let sid = &frame.session.id;
        let clicks = caption.clicks;
        if clicks.closing {
            self.app.close_session = Some(sid.clone());
        }
        if clicks.git {
            self.app.active_session = Some(sid.clone());
            self.queue_focus_tab(Tab::Terminal(sid.clone()));
            self.app.show_git_sidebar();
        }
        if clicks.split_vertical || clicks.split_horizontal || clicks.stack {
            self.app.active_session = Some(sid.clone());
            self.queue_focus_tab(Tab::Terminal(sid.clone()));
        }
        if let Some(pane) = frame.pane {
            self.queue_caption_adds(pane, clicks);
        }
        let response = &caption.response;
        if response.clicked()
            && !clicks.closing
            && !frame.editing
            && !clicks.git
            && !clicks.on_control
        {
            self.app.active_session = Some(sid.clone());
            self.queue_focus_tab(Tab::Terminal(sid.clone()));
        }
        if response.double_clicked() && !clicks.closing && !frame.editing && !clicks.on_control {
            self.app.begin_rename(sid, RenameSurface::Pane);
        }
    }

    /// The caption `+` queues the same leaf-anchored request as the leaf
    /// tab bar `+`: a stacked tab in this split, never a new top-level
    /// tab. Floating splits land in the issuing window's dock.
    fn queue_caption_adds(&mut self, pane: egui_dock::NodePath, clicks: CaptionClicks) {
        if clicks.stack {
            self.queue_add_tab(pane, None);
        }
        if clicks.split_vertical {
            self.queue_add_tab(pane, Some("right".into()));
        }
        if clicks.split_horizontal {
            self.queue_add_tab(pane, Some("down".into()));
        }
    }
}
