//! Shared Settings row grammar: label, description, and a right-hand control.
use super::*;
use terminator_core::find_executable;

pub fn matches_search(query: &str, haystacks: &[&str]) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    haystacks
        .iter()
        .any(|text| text.to_lowercase().contains(&query))
}

const LABEL_COL: f32 = 220.0;

pub fn settings_row(
    ui: &mut egui::Ui,
    label: &str,
    description: &str,
    add_control: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width(LABEL_COL);
            ui.label(label);
            if !description.is_empty() {
                ui.add(egui::Label::new(egui::RichText::new(description).weak()).wrap());
            }
        });
        ui.add_space(16.0);
        ui.vertical(|ui| {
            ui.set_min_width(ui.available_width().clamp(180.0, 360.0));
            add_control(ui);
        });
    });
    ui.add_space(8.0);
}

pub fn segmented<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    value: &mut T,
    options: &[(&str, T)],
) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (label, option) in options {
            let selected = *value == *option;
            if ui.selectable_label(selected, *label).clicked() && !selected {
                *value = *option;
                changed = true;
            }
        }
    });
    changed
}

pub fn path_field(
    ui: &mut egui::Ui,
    value: &mut String,
    #[cfg_attr(not(feature = "test-support"), allow(unused_variables))] browse_id: &str,
    browse: &mut bool,
) -> egui::Response {
    ui.horizontal(|ui| {
        let browse_width = 88.0;
        let width = (ui.available_width() - browse_width - ui.spacing().item_spacing.x).max(120.0);
        let edit = ui.add(egui::TextEdit::singleline(value).desired_width(width));
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), browse_id, edit.rect);
        if ui.button("Browse…").clicked() {
            *browse = true;
        }
        edit
    })
    .inner
}

pub fn status_badge(ui: &mut egui::Ui, label: &str, ok: bool) {
    let color = if ok {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    ui.colored_label(
        color,
        if ok {
            format!("● {label}")
        } else {
            format!("○ {label}")
        },
    );
}

#[derive(Clone, Debug)]
pub struct BinaryOption {
    pub label: String,
    pub value: String,
}

pub fn shell_options() -> Vec<BinaryOption> {
    let mut options = vec![BinaryOption {
        label: "Automatic (zsh → bash → sh)".into(),
        value: String::new(),
    }];
    let mut seen = std::collections::BTreeSet::new();
    if let Ok(shell) = std::env::var("SHELL")
        && Path::new(&shell).is_file()
        && seen.insert(shell.clone())
    {
        let name = Path::new(&shell)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        options.push(BinaryOption {
            label: format!("{name} — {shell}"),
            value: shell,
        });
    }
    for name in ["zsh", "bash", "fish", "sh"] {
        if let Some(path) = find_executable(name) {
            let value = path.display().to_string();
            if seen.insert(value.clone()) {
                options.push(BinaryOption {
                    label: format!("{name} — {value}"),
                    value,
                });
            }
        }
    }
    options
}

pub fn editor_options() -> Vec<BinaryOption> {
    let mut options = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for name in ["nvim", "neovim"] {
        if let Some(path) = find_executable(name) {
            let value = path.display().to_string();
            if seen.insert(value.clone()) {
                options.push(BinaryOption {
                    label: format!("{name} — {value}"),
                    value,
                });
            }
        }
    }
    if options.is_empty() {
        options.push(BinaryOption {
            label: "nvim (not on PATH)".into(),
            value: "nvim".into(),
        });
    }
    options
}

pub fn combo_or_custom(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut String,
    options: &[BinaryOption],
    custom: &mut bool,
) -> bool {
    let mut changed = false;
    let selected = options
        .iter()
        .find(|option| !*custom && option.value == *value)
        .map(|option| option.label.as_str())
        .unwrap_or("Custom");
    let combo = egui::ComboBox::from_id_salt(id)
        .width(ui.available_width().clamp(180.0, 320.0))
        .selected_text(selected)
        .show_ui(ui, |ui| {
            for option in options {
                let response =
                    ui.selectable_label(!*custom && *value == option.value, &option.label);
                #[cfg(test)]
                ui.ctx().data_mut(|data| {
                    data.insert_temp(egui::Id::new((id, option.label.as_str())), response.rect)
                });
                if response.clicked() {
                    *value = option.value.clone();
                    *custom = false;
                    changed = true;
                }
            }
            let response = ui.selectable_label(selected == "Custom", "Custom");
            #[cfg(test)]
            ui.ctx()
                .data_mut(|data| data.insert_temp(egui::Id::new((id, "Custom")), response.rect));
            if response.clicked() {
                *custom = true;
                changed = true;
            }
        });
    #[cfg(test)]
    ui.ctx()
        .data_mut(|data| data.insert_temp(egui::Id::new((id, "combo")), combo.response.rect));
    let _ = combo;
    changed
}

pub fn agent_binary(kind: &str) -> Option<PathBuf> {
    let names = match kind {
        "claude" => &["claude"][..],
        "codex" => &["codex"],
        "opencode" => &["opencode"],
        "muse" => &["muse"],
        "grok" => &["grok"],
        _ => &[],
    };
    names.iter().find_map(|name| find_executable(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_matches_labels_and_keywords() {
        assert!(matches_search("", &["Shell"]));
        assert!(matches_search("zsh", &["Shell override", "zsh, bash"]));
        assert!(!matches_search("nvim", &["History days"]));
    }

    #[test]
    fn shell_options_include_automatic() {
        let options = shell_options();
        assert_eq!(options[0].value, "");
        assert!(options[0].label.contains("Automatic"));
    }

    #[test]
    fn choosing_custom_keeps_the_value_and_reveals_the_field() {
        let ctx = egui::Context::default();
        let options = vec![BinaryOption {
            label: "Detected".into(),
            value: "/bin/sh".into(),
        }];
        let mut value = options[0].value.clone();
        let mut custom = false;
        let mut paint = |events| {
            let mut field_visible = false;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600.0, 400.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    combo_or_custom(ui, "test-binary", &mut value, &options, &mut custom);
                    if custom {
                        field_visible = true;
                        ui.text_edit_singleline(&mut value);
                    }
                },
            );
            output.textures_delta.clear();
            (field_visible, value.clone())
        };
        let rect = |name| {
            ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::new(("test-binary", name))))
                .unwrap()
        };
        let click = |rect: egui::Rect| {
            vec![
                egui::Event::PointerMoved(rect.center()),
                egui::Event::PointerButton {
                    pos: rect.center(),
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos: rect.center(),
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        paint(vec![]);
        paint(vec![]);
        paint(click(rect("combo")));
        paint(vec![]);
        paint(click(rect("Custom")));
        assert_eq!(paint(vec![]), (true, "/bin/sh".into()));
        paint(click(rect("combo")));
        paint(vec![]);
        paint(click(rect("Detected")));
        assert_eq!(paint(vec![]), (false, "/bin/sh".into()));
    }

    fn paint_rows(ctx: &egui::Context) -> (f32, f32) {
        let mut xs = (0.0, 0.0);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(720.0, 400.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            settings_row(ui, "Days", "short", |ui| {
                xs.0 = ui.next_widget_position().x;
                ui.label("control-a");
            });
            settings_row(
                ui,
                "A much longer label",
                "a longer description that should wrap without shifting the control column",
                |ui| {
                    xs.1 = ui.next_widget_position().x;
                    ui.label("control-b");
                },
            );
        });
        output.textures_delta.clear();
        xs
    }

    #[test]
    fn settings_controls_share_a_left_edge() {
        let ctx = egui::Context::default();
        let _ = paint_rows(&ctx);
        let (first, second) = paint_rows(&ctx);
        assert!(
            (first - second).abs() < 1.0,
            "first={first} second={second}"
        );
        assert!(first > LABEL_COL, "control column starts after the label");
    }
}
