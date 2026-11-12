use super::super::super::*;
use super::super::tabs::snapshot_rows;
use super::Viewer;

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
    bar: Option<CaptionBar>,
    clicks: CaptionClicks,
}

/// The terminal bar's own controls (none on a markdown header).
struct CaptionBar {
    branch: Option<egui::Response>,
    split_vertical: Option<egui::Response>,
    split_horizontal: Option<egui::Response>,
    more: Option<egui::Response>,
    controls_left: f32,
}

/// What the caption's buttons reported this frame.
#[derive(Clone, Copy)]
struct CaptionClicks {
    closing: bool,
    git: bool,
    split_vertical: bool,
    split_horizontal: bool,
    zoom: bool,
    resume: bool,
    on_control: bool,
}

fn clicked(response: Option<&egui::Response>) -> bool {
    response.is_some_and(egui::Response::clicked)
}

fn hovered(response: Option<&egui::Response>) -> bool {
    response.is_some_and(egui::Response::hovered)
}

impl CaptionClicks {
    fn read(
        close: Option<&egui::Response>,
        zoom: Option<&egui::Response>,
        bar: Option<&CaptionBar>,
        resume: Option<&egui::Response>,
    ) -> Self {
        let controls = bar.map_or([None; 4], |bar| {
            [
                bar.branch.as_ref(),
                bar.split_vertical.as_ref(),
                bar.split_horizontal.as_ref(),
                bar.more.as_ref(),
            ]
        });
        Self {
            resume: clicked(resume),
            zoom: clicked(zoom),
            closing: clicked(close),
            git: clicked(controls[0]),
            split_vertical: clicked(controls[1]),
            split_horizontal: clicked(controls[2]),
            on_control: hovered(close)
                || hovered(zoom)
                || hovered(resume)
                || controls.into_iter().any(hovered),
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
        // A narrow pane folds its buttons into one menu: the pane menu.
        if let Some(more) = caption.bar.as_ref().and_then(|bar| bar.more.as_ref())
            && let Some(pane) = frame.pane
        {
            egui::Popup::menu(more)
                .style(appearance::menu_style)
                .show(|ui| self.context_menu(ui, &mut Tab::Terminal(sid.clone()), pane));
        }
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
        let (response, close, zoom, bar, resume) = if frame.is_markdown {
            let header = self.markdown_header(ui, frame.session, frame.editing);
            (header.0, header.1, None, None, None)
        } else {
            let bar = self.terminal_caption_bar(ui, frame);
            (
                bar.bar,
                bar.close,
                bar.zoom,
                Some(CaptionBar {
                    branch: bar.branch,
                    split_vertical: bar.split_vertical,
                    split_horizontal: bar.split_horizontal,
                    more: bar.more,
                    controls_left: bar.controls_left,
                }),
                bar.resume,
            )
        };
        let clicks =
            CaptionClicks::read(close.as_ref(), zoom.as_ref(), bar.as_ref(), resume.as_ref());
        #[cfg(feature = "test-support")]
        if let Some(zoom) = &zoom {
            let sid = &frame.session.id;
            diagnostics::record(ui.ctx(), &format!("pane-zoom:{sid}"), zoom.rect);
        }
        Caption {
            response,
            close,
            bar,
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
        let active = self.app.active_session.as_ref() == Some(sid);
        let vertical_tip = self.app.action_tip("Split vertically", Action::SplitRight);
        let horizontal_tip = self.app.action_tip("Split horizontally", Action::SplitDown);
        // Floating windows cannot zoom; their pane already has a window.
        let zoom_tip = (self.window.is_none() && frame.pane.is_some())
            .then(|| self.app.action_tip("Zoom pane", Action::ZoomPane));
        let leading = self.app.tab_leading(sid);
        let resume = self
            .app
            .resume_chip_command(sid)
            .map(|chip| (chip.tip(), !chip.confirmed));
        let usage = self.app.agent_usage_chips(sid);
        let bar = appearance::terminal_bar(
            ui,
            appearance::TerminalBarSpec {
                title: if frame.editing || frame.lone {
                    ""
                } else {
                    &session.label
                },
                title_tint: self.app.terminal_chosen_title_color(sid),
                active,
                // A lone pane already has the active workspace tab's mark.
                focus_fill: (active && !frame.lone)
                    .then(|| appearance::color(&self.app.theme.hover)),
                branch: branch.as_deref(),
                // The status glyph already shows the state; the dot is for
                // panes without one.
                status: leading
                    .status
                    .is_none()
                    .then(|| self.app.terminal_status_color(session))
                    .flatten(),
                zoom_tip: zoom_tip.as_deref(),
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
                unread: leading.unread,
                subtitle: self.app.terminal_program_title(sid),
                resume_tip: resume.as_ref().map(|(tip, unsure)| appearance::ResumeChip {
                    tip,
                    unsure: *unsure,
                }),
            },
        );
        // Side by side, right to left: the program badge next to the
        // controls, then the agent usage chips, then the broadcast chip.
        let right = self
            .app
            .paint_program_status(ui, sid, (bar.bar.rect, bar.controls_left));
        let chips_left = appearance::paint_usage_chips(ui, bar.bar.rect, right, &usage);
        if self.app.broadcasting_to(sid) {
            let accent = appearance::color(&self.app.theme.status_waiting);
            appearance::paint_broadcast_chip(ui, bar.bar.rect, chips_left, accent);
        }
        bar
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
            .bar
            .as_ref()
            .map(|bar| bar.controls_left)
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
        if let Some(bar) = &caption.bar {
            for (name, response) in [
                ("pane-git", &bar.branch),
                ("pane-split-vertical", &bar.split_vertical),
                ("pane-split-horizontal", &bar.split_horizontal),
                ("pane-more", &bar.more),
            ] {
                if let Some(response) = response {
                    diagnostics::record(ui.ctx(), &format!("{name}:{sid}"), response.rect);
                }
            }
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
            && self.app.drag.idle()
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
        if clicks.resume {
            self.app.resume_in_shell(sid);
        }
        if clicks.git {
            self.app.focus_terminal(sid.clone());
            self.queue_focus_tab(Tab::Terminal(sid.clone()));
            self.app.show_git_sidebar();
        }
        if clicks.zoom {
            self.app
                .queue_zoom(Some(Tab::Terminal(sid.clone())), frame.pane);
        }
        if clicks.split_vertical || clicks.split_horizontal {
            self.app.focus_terminal(sid.clone());
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
            self.app.focus_terminal(sid.clone());
            self.queue_focus_tab(Tab::Terminal(sid.clone()));
        }
        if response.double_clicked() && !clicks.closing && !frame.editing && !clicks.on_control {
            self.app.begin_rename(sid, RenameSurface::Pane);
        }
    }

    /// The caption splits queue the same leaf-anchored request as the leaf
    /// tab bar menu. Floating splits land in the issuing window's dock.
    /// New tab in this split stays in the pane context menu.
    fn queue_caption_adds(&mut self, pane: egui_dock::NodePath, clicks: CaptionClicks) {
        if clicks.split_vertical {
            self.queue_add_tab(pane, Some("right".into()));
        }
        if clicks.split_horizontal {
            self.queue_add_tab(pane, Some("down".into()));
        }
    }
}
