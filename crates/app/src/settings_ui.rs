//! Settings window and appearance preview.
use super::*;

impl App {
    pub(super) fn open_settings(&mut self) {
        self.settings_draft = self.state.settings.clone();
        self.editor_preset = external_editor::selected(&self.settings_draft);
        self.theme_draft = self.theme_committed.clone();
        self.theme_conflict = false;
        let _ = self.jobs.send(Job::HookStatus);
        self.settings_open = true;
    }
    pub(super) fn settings(&mut self, ctx: &egui::Context) {
        let mut open = true;
        let height = (ctx.content_rect().height() - 160.0).clamp(220.0, 580.0);
        self.popups.window(ctx, "Settings").open(&mut open).collapsible(false).default_size([740.0,height+90.0]).show(ctx,|ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(142.0);ui.set_min_height(height);ui.spacing_mut().item_spacing.y=4.0;
                    for (index,label,icon) in [(0,"Appearance","Settings2"),(1,"Terminal & Editor","Terminal"),(2,"Notifications","PanelsTopLeft"),(3,"History","FileText"),(4,"Shortcuts","SquareDashed"),(5,"Agent Hooks","GitBranch"),(6,"Updates","Settings2")] {
                        let section = appearance::row(ui,label,icon,self.settings_section==index,30.0,"",ui.visuals().weak_text_color());
                        #[cfg(feature = "test-support")]
                        diagnostics::record(ui.ctx(), &format!("settings-section:{label}"), section.rect);
                        if section.clicked(){self.settings_section=index;}
                    }
                });
                let divider=ui.cursor().min;
                ui.painter().line_segment([divider,divider+egui::vec2(0.0,height)],ui.visuals().widgets.noninteractive.bg_stroke);
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.set_width(540.0);
                    egui::ScrollArea::vertical().id_salt(("settings-section",self.settings_section)).max_height(height).show(ui,|ui| {
                        match self.settings_section {
                            6 => {
                                self.installation_settings(ui);
                                ui.add_space(12.0);
                                ui.separator();
                                self.updater.settings(ui);
                            },
                            0 => {
                ui.heading("Appearance");
                if self.theme_conflict {
                    ui.colored_label(appearance::color(&self.theme.status_failed), "Configuration changed externally. Reload before applying.");
                    if ui.button("Reload appearance").clicked() { self.theme_draft = self.theme_committed.clone(); self.theme_conflict = false; }
                }
                ui.weak("Preview colors and borders. Apply saves your changes.");
                ui.add_space(12.0);
                for (group,title,expanded) in [(0,"Interface",true),(1,"Terminal",false),(2,"Git status",false),(3,"Agent status",false)] {
                    egui::CollapsingHeader::new(title).id_salt(("appearance-group",group)).default_open(expanded).show(ui,|ui| {
                        egui::Grid::new(("appearance-fields",group)).num_columns(2).min_col_width(155.0).spacing([20.0,10.0]).show(ui,|ui| {
                            for (name,value) in self.theme_draft.colors_mut() {
                                let category=if name.starts_with("terminal_"){1}else if name.starts_with("git_"){2}else if name.starts_with("status_"){3}else{0};
                                if category!=group {continue;}
                                let label=match name {"window"=>"Workspace background".into(),"surface"=>"Sidebar background".into(),"hover"=>"Row highlight".into(),"text"=>"Primary text".into(),"secondary"=>"Secondary text".into(),_=>{let label=name.replace('_'," ");let mut chars=label.chars();chars.next().unwrap().to_uppercase().to_string()+chars.as_str()}};
                                ui.label(label);
                                ui.horizontal(|ui| {
                                    let mut color=terminator_core::appearance::rgb(value).unwrap_or([0,0,0]);
                                    if ui.color_edit_button_srgb(&mut color).changed() {*value=format!("#{:02X}{:02X}{:02X}",color[0],color[1],color[2]);}
                                    ui.add_sized([108.0,28.0],egui::TextEdit::singleline(value).font(egui::TextStyle::Monospace));
                                    if terminator_core::appearance::rgb(value).is_err(){ui.colored_label(appearance::color(&self.theme.status_failed),"#RRGGBB");}
                                });ui.end_row();
                            }
                            if group==0 {
                                ui.label("Border width");ui.add(egui::DragValue::new(&mut self.theme_draft.border_width).range(0.0..=8.0).speed(0.1).suffix(" pt"));ui.end_row();
                                ui.label("Pane divider width");ui.add(egui::DragValue::new(&mut self.theme_draft.pane_divider_width).range(1.0..=24.0).suffix(" pt"));ui.end_row();
                            }
                        });
                    });
                }
                ui.add_space(8.0);
                if ui.button("Reset appearance").clicked() { self.theme_draft = AppearanceConfig::default(); }

                            },
                            1 => {
                ui.heading("Terminal & Editor");
                egui::Grid::new("terminal-settings").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                    ui.label("Shell override"); ui.add(egui::TextEdit::singleline(&mut self.settings_draft.shell).hint_text("Automatic: zsh → bash → sh").desired_width(250.0)); ui.end_row();
                    ui.label("Pull requests"); ui.add_enabled(self.state.capabilities.iter().any(|c|c==METADATA_SETTINGS_CAPABILITY),egui::Checkbox::new(&mut self.settings_draft.pr_metadata,"Fetch PR metadata with GitHub CLI")); ui.end_row();
                    ui.label("Font size"); ui.add(egui::Slider::new(&mut self.settings_draft.font_size, 9.0..=32.0)); ui.end_row();
                    ui.label("Editor mode"); egui::ComboBox::from_id_salt("editor-mode").selected_text(format!("{:?}", self.settings_draft.editor_mode)).show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.settings_draft.editor_mode, EditorMode::Embedded, "Embedded Neovim");
                        ui.selectable_value(&mut self.settings_draft.editor_mode, EditorMode::Terminal, "Terminal editor");
                        ui.selectable_value(&mut self.settings_draft.editor_mode, EditorMode::External, "External editor");
                    }); ui.end_row();
                    ui.label("Editor executable"); ui.add(egui::TextEdit::singleline(&mut self.settings_draft.editor_program).desired_width(250.0)); ui.end_row();
                    ui.label("External editor");
                    egui::ComboBox::from_id_salt("external-preset").selected_text(external_editor::PRESETS[self.editor_preset]).show_ui(ui, |ui| {
                        for (index, label) in external_editor::PRESETS.iter().enumerate() {
                            if ui.selectable_value(&mut self.editor_preset, index, *label).changed() && index != external_editor::CUSTOM {
                                (self.settings_draft.external_editor, self.settings_draft.external_args) = external_editor::preset(index);
                            }
                        }
                    }); ui.end_row();
                    if self.editor_preset == external_editor::CUSTOM {
                        ui.label("Executable"); let _executable = ui.text_edit_singleline(&mut self.settings_draft.external_editor);
                        #[cfg(feature = "test-support")]
                        diagnostics::record(ui.ctx(), "external-program", _executable.rect);
                        ui.end_row();
                        ui.label("Arguments"); ui.vertical(|ui| {
                            let mut remove = None;
                            for (index, arg) in self.settings_draft.external_args.iter_mut().enumerate() {
                                ui.horizontal(|ui| { ui.text_edit_singleline(arg); if ui.small_button("×").clicked() { remove = Some(index); } });
                            }
                            if let Some(index) = remove { self.settings_draft.external_args.remove(index); }
                            if ui.button("Add argument").clicked() { self.settings_draft.external_args.push(String::new()); }
                            ui.weak("One argument per row. The file path is appended automatically.");
                        }); ui.end_row();
                    }
                    ui.label("Test draft"); if ui.add_enabled(!self.picker_active, egui::Button::new("Choose file and test…")).clicked() { self.test_editor = true; } ui.end_row();
                });

                            },
                            2 => {
                ui.heading("Notifications");
                egui::Grid::new("notification-settings").num_columns(3).show(ui, |ui| {
                    for state in [AgentState::WaitingInput, AgentState::WaitingPermission, AgentState::Completed, AgentState::Failed] {
                        ui.label(state.label());
                        for (events, label) in [(&mut self.settings_draft.events, "In app"), (&mut self.settings_draft.os_events, "OS when unfocused")] {
                            let mut enabled = events.contains(&state);
                            if ui.checkbox(&mut enabled, label).changed() { if enabled { events.insert(state); } else { events.remove(&state); } }
                        }
                        ui.end_row();
                    }
                });
                egui::ComboBox::from_label("Dismiss notifications").selected_text(format!("{:?}", self.settings_draft.dismissal)).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.settings_draft.dismissal, Dismissal::OnFocus, "When opening terminal");
                    ui.selectable_value(&mut self.settings_draft.dismissal, Dismissal::OnResolve, "When request resolves");
                    ui.selectable_value(&mut self.settings_draft.dismissal, Dismissal::Manual, "Manually");
                });
                ui.checkbox(&mut self.settings_draft.notifications_side, "Place notifications at the side");
                ui.add_enabled_ui(self.state.capabilities.iter().any(|c|c==TERMINAL_NOTICES_CAPABILITY),|ui| {
                    ui.checkbox(&mut self.settings_draft.terminal_notifications,"Show terminal OSC notifications");
                    ui.checkbox(&mut self.settings_draft.terminal_notifications_os,"Also send terminal notifications to the desktop");
                });

                            },
                            3 => {
                ui.heading("History");
                egui::Grid::new("history-settings").num_columns(2).show(ui, |ui| {
                    ui.label("Days"); ui.add(egui::DragValue::new(&mut self.settings_draft.history_days).range(1..=3650)); ui.end_row();
                    ui.label("MiB per session"); ui.add(egui::DragValue::new(&mut self.settings_draft.session_mib).range(1..=4096)); ui.end_row();
                    ui.label("MiB total"); ui.add(egui::DragValue::new(&mut self.settings_draft.total_mib).range(1..=65536)); ui.end_row();
                });
                ui.weak("Limits apply to saved output; records and resume commands remain.");

                            },
                            4 => {
                ui.heading("Shortcuts");
                egui::Grid::new("shortcut-settings").num_columns(2).show(ui, |ui| {
                    for (action, key) in &mut self.settings_draft.keybindings { ui.label(action); ui.add(egui::TextEdit::singleline(key).desired_width(250.0)); ui.end_row(); }
                });
                ui.weak("command = Cmd on macOS, Ctrl+Shift on Linux; combine with shift/alt and a key.");

                            },
                            5 => {
                ui.heading("Agent Hooks");
                egui::Grid::new("hook-settings").num_columns(4).show(ui, |ui| {
                    for kind in terminator_integrations::AGENTS {
                        let installed = self.hook_status.get(kind).copied();
                        ui.label(kind); ui.weak(match installed {Some(true)=>"Configured",Some(false)=>"Not configured",None=>"Checking…"});
                        if ui.button(if installed == Some(true) { "Repair" } else { "Install" }).clicked() { let _ = self.jobs.send(Job::Install(kind.to_string(), false)); let _ = self.jobs.send(Job::HookStatus); }
                        if ui.add_enabled(installed == Some(true), egui::Button::new("Remove")).clicked() { let _ = self.jobs.send(Job::Install(kind.to_string(), true)); let _ = self.jobs.send(Job::HookStatus); }
                        ui.end_row();
                    }
                });
                ui.weak("Agents are launched manually. See docs/INTEGRATIONS.md for event limitations.");

                            },
                            _=>{}
                        }
                    });
                });
            });
            ui.separator();
            let validation = self.theme_draft.validate().and_then(|()| self.settings_draft.validate());
            if let Err(e) = &validation { ui.colored_label(appearance::color(&self.theme.status_failed), e.to_string()); }
            ui.horizontal(|ui| {
                if ui.add_enabled(validation.is_ok() && !self.theme_conflict, egui::Button::new("Apply")).clicked() {
                    let _ = self.jobs.send(Job::SaveAppearance(Box::new(self.theme_draft.clone()), self.theme_source.clone()));
                    self.send(Request::Settings(self.settings_draft.clone()));
                }
                let cancel = ui.button("Cancel");
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "settings-cancel", cancel.rect);
                if cancel.clicked() { self.settings_open = false; }
            });
        });
        self.settings_open &= open;
        self.preview_appearance(ctx);
    }
    pub(super) fn preview_appearance(&mut self, ctx: &egui::Context) {
        let next = if !self.settings_open {
            self.theme_committed.clone()
        } else if self.theme_draft.validate().is_ok() {
            self.theme_draft.clone()
        } else {
            return;
        };
        if self.theme != next {
            self.theme = next;
            appearance::apply(ctx, &self.theme);
        }
    }
}
