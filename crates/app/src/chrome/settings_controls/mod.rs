//! Shared Settings row grammar: label, description, and a right-hand control.
use crate::*;
use terminator_core::find_executable;

/// Whether every word of `query` is in `haystacks` (any order, any case).
/// An empty query matches everything.
pub fn matches_search(query: &str, haystacks: &[&str]) -> bool {
    let text = haystacks.join(" ").to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|word| text.contains(word))
}

mod row;
pub use row::{
    FIELD_WIDTH, SettingsRowInput, button_width, control_column, description_line, settings_row,
};

/// The one reason a control that needs a newer session service is off.
pub const NEEDS_SERVICE: &str = "Needs the updated session service.";

/// One caption line under a control that the running session service
/// cannot use yet: an icon, [`NEEDS_SERVICE`], and an Open Updates link.
/// Returns true when the link was clicked.
pub fn needs_service_caption(ui: &mut egui::Ui) -> bool {
    let tint = ui.visuals().weak_text_color();
    ui.horizontal(|ui| {
        ui.add(
            egui::Image::new(crate::icons::source("RefreshCw"))
                .tint(tint)
                .fit_to_exact_size(egui::Vec2::splat(appearance::FONT_CAPTION)),
        );
        ui.label(
            egui::RichText::new(NEEDS_SERVICE)
                .size(appearance::FONT_CAPTION)
                .color(tint),
        );
        ui.link(egui::RichText::new("Open Updates").size(appearance::FONT_CAPTION))
            .clicked()
    })
    .inner
}

/// A group title inside a section, with a rule under it.
pub fn sub_heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new(text)
            .strong()
            .color(ui.visuals().weak_text_color()),
    );
    ui.separator();
}

/// Mutually exclusive choices in one framed control, so the options read
/// as one setting. They wrap to more lines when the card is narrow.
pub fn segmented<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    value: &mut T,
    options: &[(&str, T)],
) -> bool {
    let mut changed = false;
    egui::Frame::new()
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(egui::CornerRadius::same(appearance::WIDGET_RADIUS))
        .inner_margin(egui::Margin::same(2))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
                for (label, option) in options {
                    let selected = *value == *option;
                    if ui.selectable_label(selected, *label).clicked() && !selected {
                        *value = *option;
                        changed = true;
                    }
                }
            });
        });
    changed
}

pub struct PathFieldInput<'a> {
    pub ui: &'a mut egui::Ui,
    pub value: &'a mut String,
    pub browse_id: &'a str,
    pub browse: &'a mut bool,
}

pub fn path_field(input: PathFieldInput<'_>) -> egui::Response {
    #[cfg_attr(
        not(feature = "test-support"),
        expect(unused_variables, reason = "only fixtures address the browse button")
    )]
    let PathFieldInput {
        ui,
        value,
        browse_id,
        browse,
    } = input;
    ui.horizontal(|ui| {
        let browse_width = 88.0;
        let width = (ui.available_width() - browse_width - ui.spacing().item_spacing.x).max(40.0);
        let edit = ui.add(appearance::singleline(value).desired_width(width));
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

pub struct ComboOrCustomInput<'a> {
    pub ui: &'a mut egui::Ui,
    pub id: &'a str,
    pub value: &'a mut String,
    pub options: &'a [BinaryOption],
    pub custom: &'a mut bool,
}

pub fn combo_or_custom(input: ComboOrCustomInput<'_>) -> bool {
    let ComboOrCustomInput {
        ui,
        id,
        value,
        options,
        custom,
    } = input;
    let mut changed = false;
    let selected = options
        .iter()
        .find(|option| !*custom && option.value == *value)
        .map(|option| option.label.as_str())
        .unwrap_or("Custom");
    let combo = egui::ComboBox::from_id_salt(id)
        .width(ui.available_width().clamp(60.0, 320.0))
        .truncate()
        .selected_text(selected)
        .show_ui(ui, |ui| {
            for option in options {
                let response =
                    ui.selectable_label(!*custom && *value == option.value, &option.label);
                #[cfg(test)]
                ui.ctx().data_mut(|data| {
                    data.insert_temp(egui::Id::unique((id, option.label.as_str())), response.rect)
                });
                if response.clicked() {
                    value.clone_from(&option.value);
                    *custom = false;
                    changed = true;
                }
            }
            let response = ui.selectable_label(selected == "Custom", "Custom");
            #[cfg(test)]
            ui.ctx()
                .data_mut(|data| data.insert_temp(egui::Id::unique((id, "Custom")), response.rect));
            if response.clicked() {
                *custom = true;
                changed = true;
            }
        });
    #[cfg(test)]
    ui.ctx()
        .data_mut(|data| data.insert_temp(egui::Id::unique((id, "combo")), combo.response.rect));
    let _ = combo;
    changed
}

pub use terminator_core::agents::find_installed as agent_binary;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrow_settings_rows_put_the_control_below_the_label() {
        let ctx = egui::Context::default();
        let mut control = egui::Rect::NOTHING;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(280.0, 300.0),
                )),
                ..Default::default()
            },
            |ui| {
                settings_row(SettingsRowInput {
                    ui,
                    label: "Shell override",
                    description: "Choose the shell for new terminals.",
                    add_control: |ui| {
                        control = ui.button("Browse…").rect;
                    },
                })
            },
        );
        output.textures_delta.clear();
        // On the line under the label, not beside it.
        assert!(control.top() > 12.0, "{control:?}");
        assert!(control.right() <= 280.0);
        assert!(control.left() < 20.0);
    }

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
                    combo_or_custom(ComboOrCustomInput {
                        ui,
                        id: "test-binary",
                        value: &mut value,
                        options: &options,
                        custom: &mut custom,
                    });
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
            ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("test-binary", name))))
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
}
