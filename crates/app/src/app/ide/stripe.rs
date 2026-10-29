//! Tool stripes on both window edges: one button per sidebar tool plus the
//! command palette search pinned at the bottom.

use super::StripeTool;
use crate::*;

/// Stripe buttons top to bottom: tool, Lucide icon, caption.
const STRIPE_TOOLS: [(StripeTool, &str, &str); 10] = [
    (StripeTool::Projects, "Folder", "Projects"),
    (StripeTool::Tool(SidebarTool::Explorer), "Files", "Explorer"),
    (StripeTool::Tool(SidebarTool::Agents), "Bell", "Agents"),
    (StripeTool::Tool(SidebarTool::Git), "GitBranch", "Git"),
    (StripeTool::Tool(SidebarTool::History), "History", "History"),
    (StripeTool::Tool(SidebarTool::Info), "Info", "Info"),
    (
        StripeTool::Tool(SidebarTool::Problems),
        "CircleAlert",
        "Problems",
    ),
    (
        StripeTool::Tool(SidebarTool::Structure),
        "ListTree",
        "Structure",
    ),
    (StripeTool::Tool(SidebarTool::Vnc), "Monitor", "VNC"),
    (StripeTool::Tool(SidebarTool::Hosts), "Globe", "Hosts"),
];

#[cfg(feature = "test-support")]
struct RecordStripeToolInput<'a> {
    ui: &'a egui::Ui,
    right: bool,
    label: &'a str,
    rect: egui::Rect,
}

impl App {
    /// Both stripes carry every tool; clicking renders it in that side's
    /// sidebar. Registered before its sidebar so it stays on the window edge.
    pub(super) fn tool_stripe(&mut self, ui: &mut egui::Ui, right: bool) {
        let panel = if right {
            egui::Panel::right("tool-stripe-right")
        } else {
            egui::Panel::left("tool-stripe-left")
        };
        panel
            .exact_size(44.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(appearance::color(&self.theme.window))
                    .inner_margin(egui::Margin::symmetric(2, 6)),
            )
            .show(ui, |ui| {
                let bounds = ui.available_rect_before_wrap();
                let search_height = bounds.height().clamp(0.0, 46.0);
                let search_rect = egui::Rect::from_min_max(
                    egui::pos2(bounds.left(), bounds.bottom() - search_height),
                    bounds.max,
                );
                let tools_rect = egui::Rect::from_min_max(
                    bounds.min,
                    egui::pos2(
                        bounds.right(),
                        (search_rect.top() - ui.spacing().item_spacing.y).max(bounds.top()),
                    ),
                );
                ui.allocate_rect(bounds, egui::Sense::hover());
                self.stripe_tools(ui, right, tools_rect);
                self.stripe_search(ui, right, search_rect);
            });
    }

    /// The scrolling tool buttons, clipped above the search button.
    fn stripe_tools(&mut self, ui: &mut egui::Ui, right: bool, tools_rect: egui::Rect) {
        let mut tools_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("stripe-tools-region", right))
                .max_rect(tools_rect)
                .layout(egui::Layout::top_down(egui::Align::Center)),
        );
        tools_ui.set_clip_rect(ui.clip_rect().intersect(tools_rect));
        egui::ScrollArea::vertical()
            .id_salt(("stripe-tools", right))
            .min_scrolled_height(0.0)
            .max_height(tools_rect.height())
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(&mut tools_ui, |ui| {
                for entry in STRIPE_TOOLS {
                    self.stripe_tool_button(ui, right, entry);
                }
            });
    }

    fn stripe_tool_button(
        &mut self,
        ui: &mut egui::Ui,
        right: bool,
        (tool, icon, label): (StripeTool, &str, &str),
    ) {
        let selected = self.stripe_selected(right, tool);
        let badge = self.stripe_badge(tool);
        // Keep the narrow caption and fixture identity stable.
        let caption = if tool == StripeTool::Tool(SidebarTool::Git)
            && self.context.as_ref().is_some_and(|c| c.perforce.is_some())
        {
            "P4"
        } else {
            label
        };
        let response = appearance::stripe_button(appearance::StripeButtonInput {
            ui,
            icon,
            label: caption,
            selected,
            badge,
        });
        let response = if tool == StripeTool::Tool(SidebarTool::Git) {
            response.on_hover_text("Git / Perforce")
        } else {
            response
        };
        #[cfg(feature = "test-support")]
        Self::record_stripe_tool(RecordStripeToolInput {
            ui,
            right,
            label,
            rect: response.rect,
        });
        if response.clicked() {
            self.stripe_activate(right, tool);
        }
    }

    /// Attention badge color for the Agents button: waiting beats unread.
    fn stripe_badge(&self, tool: StripeTool) -> Option<egui::Color32> {
        if tool != StripeTool::Tool(SidebarTool::Agents) {
            return None;
        }
        let (waiting, unread) = self.attention_counts();
        (waiting.saturating_add(unread) > 0).then(|| {
            appearance::color(if waiting > 0 {
                &self.theme.status_waiting
            } else {
                &self.theme.text
            })
        })
    }

    #[cfg(feature = "test-support")]
    fn record_stripe_tool(input: RecordStripeToolInput<'_>) {
        let RecordStripeToolInput {
            ui,
            right,
            label,
            rect,
        } = input;
        diagnostics::record(
            ui.ctx(),
            &format!("stripe-{}-{label}", if right { "right" } else { "left" }),
            rect,
        );
        if !right {
            let legacy = match label {
                "Projects" => Some("stripe-projects"),
                "Agents" => Some("stripe-agents"),
                _ => None,
            };
            if let Some(name) = legacy {
                diagnostics::record(ui.ctx(), name, rect);
            }
        }
        if right {
            diagnostics::record(ui.ctx(), &format!("stripe-tool-{label}"), rect);
        }
    }

    fn stripe_search(&mut self, ui: &mut egui::Ui, right: bool, search_rect: egui::Rect) {
        let mut search_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("stripe-search-region", right))
                .max_rect(search_rect)
                .layout(egui::Layout::top_down(egui::Align::Center)),
        );
        search_ui.set_clip_rect(ui.clip_rect().intersect(search_rect));
        search_ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            let search = appearance::stripe_button(appearance::StripeButtonInput {
                ui,
                icon: "Search",
                label: "Search",
                selected: false,
                badge: None,
            });
            #[cfg(feature = "test-support")]
            diagnostics::record(
                ui.ctx(),
                &format!("stripe-{}-Search", if right { "right" } else { "left" }),
                search.rect,
            );
            #[cfg(feature = "test-support")]
            if !right {
                diagnostics::record(ui.ctx(), "stripe-search", search.rect);
            }
            if search.clicked() {
                self.open_command_palette();
            }
        });
    }

    fn stripe_selected(&self, right: bool, tool: StripeTool) -> bool {
        if right {
            self.preferences.visible
                && match tool {
                    StripeTool::Projects => self.preferences.right_projects,
                    StripeTool::Tool(tool) => {
                        !self.preferences.right_projects && self.preferences.tool == tool
                    }
                }
        } else {
            self.preferences.left_visible
                && match tool {
                    StripeTool::Projects => {
                        self.preferences.left_tool.is_none() && !self.preferences.left_agents
                    }
                    StripeTool::Tool(SidebarTool::Agents) => self.left_agents_shown(),
                    StripeTool::Tool(tool) => self.preferences.left_tool == Some(tool),
                }
        }
    }

    fn stripe_activate(&mut self, right: bool, tool: StripeTool) {
        if self.stripe_selected(right, tool) {
            // Clicking the active tool hides that side's sidebar.
            if right {
                self.preferences.visible = false;
            } else {
                self.preferences.left_visible = false;
            }
            return;
        }
        if right {
            match tool {
                StripeTool::Projects => self.preferences.right_projects = true,
                StripeTool::Tool(tool) => {
                    self.preferences.right_projects = false;
                    self.preferences.tool = tool;
                }
            }
            self.preferences.visible = true;
        } else {
            match tool {
                StripeTool::Projects => {
                    self.preferences.left_tool = None;
                    self.preferences.left_agents = false;
                }
                StripeTool::Tool(SidebarTool::Agents) => {
                    self.preferences.left_tool = None;
                    if !self.preferences.left_agents {
                        self.preferences.agents_tab = AgentsTab::NeedsAttention;
                    }
                    self.preferences.left_agents = true;
                }
                StripeTool::Tool(tool) => self.preferences.left_tool = Some(tool),
            }
            self.preferences.left_visible = true;
        }
    }
}

#[cfg(all(test, feature = "test-support"))]
mod stripe_layout_tests {
    use crate::*;

    #[test]
    fn short_tool_stripes_clip_tools_above_the_search_button() {
        let (mut app, ctx, _dir) = crate::app::tests::support::fixture();
        for height in [650.0, 220.0, 90.0, 400.0, 70.0, 650.0] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900.0, height),
                    )),
                    ..Default::default()
                },
                |ui| app.tool_stripe(ui, false),
            );
            output.textures_delta.clear();
            let search = ctx.data(|data| {
                data.get_temp::<egui::Rect>(egui::Id::unique((
                    "fixture-target",
                    "stripe-left-Search",
                )))
                .expect("Search remains available")
            });
            assert!(search.bottom() <= height);
            assert_eq!(search.height(), 46.0);
            for clipped in &output.shapes {
                if let egui::epaint::Shape::Text(text) = &clipped.shape
                    && matches!(text.galley.text(), "VNC" | "Structure")
                {
                    assert!(
                        clipped.clip_rect.bottom() <= search.top(),
                        "height={height}"
                    );
                }
            }
        }
    }
}
