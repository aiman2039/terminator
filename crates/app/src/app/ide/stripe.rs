//! Compact tool stripes: pinned and active tools, More, and command search.

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
    /// Either edge can host every tool through More; pins stay on that edge.
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
                    .fill(appearance::color(&self.theme.surface))
                    .inner_margin(egui::Margin::symmetric(2, 6)),
            )
            .show(ui, |ui| {
                let bounds = ui.available_rect_before_wrap();
                // Search Everywhere has one stripe home, the left stripe;
                // the header centre has the other.
                let search_height = if right {
                    0.0
                } else {
                    bounds.height().clamp(0.0, 46.0)
                };
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
                if !right {
                    self.stripe_search(ui, search_rect);
                }
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
                    if self.stripe_pinned(right, entry.2) || self.stripe_selected(right, entry.0) {
                        self.stripe_tool_button(ui, right, entry);
                    }
                }
                self.stripe_more(ui, right);
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
        let mut tip = self.stripe_tip(tool, label);
        if tool == StripeTool::Tool(SidebarTool::Explorer) {
            // The Explorer tip names the folder it shows.
            let folder = self.explorer_tooltip();
            tip.push_str(folder.strip_prefix("Explorer").unwrap_or(&folder));
        }
        let response = appearance::stripe_button(appearance::StripeButtonInput {
            ui,
            icon,
            label: caption,
            tip: &tip,
            selected,
            badge,
        });
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
        appearance::context_menu(&response, |ui| {
            let pinned = self.stripe_pinned(right, label);
            let title = if pinned { "Unpin tool" } else { "Pin tool" };
            if appearance::menu_action(ui, true, title, "Pin").clicked() {
                self.set_stripe_pin(right, label, !pinned);
            }
        });
    }

    fn stripe_pinned(&self, right: bool, label: &str) -> bool {
        let pins = if right {
            &self.preferences.right_tool_pins
        } else {
            &self.preferences.left_tool_pins
        };
        pins.iter().any(|pin| pin == label)
    }

    fn set_stripe_pin(&mut self, right: bool, label: &str, pinned: bool) {
        let pins = if right {
            &mut self.preferences.right_tool_pins
        } else {
            &mut self.preferences.left_tool_pins
        };
        pins.retain(|pin| pin != label);
        if pinned {
            pins.push(label.to_owned());
        }
    }

    fn stripe_more(&mut self, ui: &mut egui::Ui, right: bool) {
        let response = appearance::stripe_button(appearance::StripeButtonInput {
            ui,
            icon: "EllipsisVertical",
            label: "More",
            tip: "More tools and pinned tools",
            selected: false,
            badge: None,
        });
        #[cfg(feature = "test-support")]
        diagnostics::record(
            ui.ctx(),
            &format!("stripe-{}-More", if right { "right" } else { "left" }),
            response.rect,
        );
        egui::Popup::menu(&response)
            .style(appearance::menu_style)
            .show(|ui| {
                for (tool, icon, label) in STRIPE_TOOLS {
                    let tip = self.stripe_tip(tool, label);
                    if appearance::menu_action(ui, true, label, icon)
                        .on_hover_text(tip)
                        .clicked()
                    {
                        self.stripe_activate(right, tool);
                    }
                }
                ui.separator();
                appearance::menu_button(ui, "Pinned tools", |ui| {
                    for (_, _, label) in STRIPE_TOOLS {
                        let mut pinned = self.stripe_pinned(right, label);
                        let response = ui.checkbox(&mut pinned, label);
                        #[cfg(feature = "test-support")]
                        diagnostics::record_ui(ui, &format!("stripe-pin:{label}"), response.rect);
                        if response.changed() {
                            self.set_stripe_pin(right, label, pinned);
                        }
                    }
                });
            });
    }

    /// Hover text: the tool name and the key that toggles it.
    pub(crate) fn stripe_tip(&self, tool: StripeTool, label: &str) -> String {
        match tool {
            StripeTool::Tool(SidebarTool::Git) => self.tool_tip("Git / Perforce", SidebarTool::Git),
            StripeTool::Tool(tool) => self.tool_tip(label, tool),
            StripeTool::Projects => label.to_owned(),
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

    fn stripe_search(&mut self, ui: &mut egui::Ui, search_rect: egui::Rect) {
        let mut search_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("stripe-search-region")
                .max_rect(search_rect)
                .layout(egui::Layout::top_down(egui::Align::Center)),
        );
        search_ui.set_clip_rect(ui.clip_rect().intersect(search_rect));
        let tip = appearance::shortcut_tip(
            Action::OpenPalette.label(),
            &self.shortcut_label(Action::OpenPalette),
        );
        search_ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            let search = appearance::stripe_button(appearance::StripeButtonInput {
                ui,
                icon: "Search",
                label: "Search",
                tip: &tip,
                selected: false,
                badge: None,
            });
            #[cfg(feature = "test-support")]
            {
                diagnostics::record(ui.ctx(), "stripe-left-Search", search.rect);
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
                        self.preferences.left_content() == LeftContent::Projects
                    }
                    StripeTool::Tool(tool) => {
                        self.preferences.left_content() == LeftContent::of_tool(tool)
                    }
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
        match tool {
            StripeTool::Tool(SidebarTool::Vnc) => self.show_vnc_page(),
            StripeTool::Tool(SidebarTool::Hosts) => self.show_ssh_page(),
            _ => {}
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
                StripeTool::Projects => self.preferences.set_left_content(LeftContent::Projects),
                StripeTool::Tool(SidebarTool::Agents) => self.show_left_agents(),
                StripeTool::Tool(tool) => {
                    self.preferences.set_left_content(LeftContent::Tool(tool))
                }
            }
            self.preferences.left_visible = true;
        }
    }
}

#[cfg(all(test, feature = "test-support"))]
#[path = "stripe_tests.rs"]
mod stripe_layout_tests;
