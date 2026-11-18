use super::*;

const ROW_H: f32 = 28.0;

pub(super) fn playlist_panel(app: &mut App, ui: &mut Ui, project: &str) {
    playlist_header(app, ui);
    divider(ui);
    let extra = app.player.stack.is_some() || app.player.url_prompt;
    let footer_h = if extra { 100.0 } else { 44.0 };
    let height = (ui.available_height() - footer_h).max(120.0);
    let (list, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    playlist_rows(PlaylistRowsInput {
        app,
        ui,
        rect: list,
        project,
    });
    divider(ui);
    playlist_footer(app, ui, project);
}

/// The playlist switcher, and the track count with the total time.
fn playlist_header(app: &mut App, ui: &mut Ui) {
    let names: Vec<String> = app
        .preferences
        .playlists
        .iter()
        .map(|playlist| playlist.name.clone())
        .collect();
    let selected = app.preferences.selected_playlist.clone();
    let title = if selected.is_empty() {
        "Playlist"
    } else {
        selected.as_str()
    };
    let summary = playlist_summary(app);
    let mut pick = None;
    ui.horizontal(|ui| {
        appearance::text_menu_button(ui, title, |ui| {
            for name in &names {
                if ui.selectable_label(*name == selected, name).clicked() {
                    pick = Some(name.clone());
                    ui.close();
                }
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.weak(egui::RichText::new(summary).size(appearance::FONT_SMALL));
        });
    });
    if let Some(name) = pick {
        app.player.confirm = None;
        app.player_switch_list(name);
    }
}

pub(super) fn track_count(count: usize) -> String {
    if count == 1 {
        "1 track".into()
    } else {
        format!("{count} tracks")
    }
}

/// `12 tracks · 41:07`; the time counts only the durations read so far.
pub(super) fn playlist_summary(app: &App) -> String {
    let tracks = app.preferences.selected_tracks();
    let total: Duration = tracks
        .iter()
        .filter_map(|path| app.player.durations.get(path).copied())
        .fold(Duration::ZERO, Duration::saturating_add);
    let count = track_count(tracks.len());
    if total.is_zero() {
        count
    } else {
        format!("{count} · {}", format_clock(total))
    }
}

fn empty_playlist(app: &mut App, ui: &mut Ui) {
    ui.weak("No tracks yet.");
    ui.horizontal_wrapped(|ui| {
        if ui.button("Add files…").clicked() {
            app.request_pick(PickKind::AudioFiles);
        }
        if ui.button("Add folder…").clicked() {
            app.request_pick(PickKind::AudioDir);
        }
    });
}

pub(super) struct PlaylistRowsInput<'a> {
    pub(super) app: &'a mut App,
    pub(super) ui: &'a mut Ui,
    pub(super) rect: Rect,
    pub(super) project: &'a str,
}

pub(super) fn playlist_rows(input: PlaylistRowsInput<'_>) {
    let PlaylistRowsInput {
        app,
        ui,
        rect,
        project,
    } = input;
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min))
            .id_salt("player-rows"),
    );
    if app.preferences.selected_tracks().is_empty() {
        empty_playlist(app, &mut child);
        return;
    }
    let mut play = None;
    let mut click = None;
    let colors = RowColors {
        selection: child.visuals().selection.bg_fill,
        accent: child.visuals().selection.stroke.color,
        text: child.visuals().text_color(),
        muted: child.visuals().weak_text_color(),
        hover: child.visuals().widgets.hovered.bg_fill,
    };
    let tracks = app.preferences.selected_tracks();
    egui::ScrollArea::vertical()
        .id_salt("player-playlist")
        .auto_shrink([false, false])
        .show_rows(&mut child, ROW_H, tracks.len(), |ui, range| {
            for index in range {
                let Some(path) = tracks.get(index) else {
                    continue;
                };
                let response = playlist_row(PlaylistRowInput {
                    ui,
                    player: &app.player,
                    index,
                    path,
                    colors,
                });
                if response.double_clicked() {
                    play = Some(index);
                } else if response.clicked() {
                    let modifiers = ui.input(|i| i.modifiers);
                    click = Some((index, modifiers.shift, modifiers.command || modifiers.ctrl));
                }
                response.on_hover_text(path.display().to_string());
            }
        });
    if let Some(index) = play {
        app.player_play_index(project, index);
    } else if let Some((index, shift, ctrl)) = click {
        app.player.click_track(index, shift, ctrl);
    }
    if !ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.key_pressed(egui::Key::Delete)) {
        app.player_rem_selected();
    }
}

#[derive(Clone, Copy)]
struct RowColors {
    selection: egui::Color32,
    accent: egui::Color32,
    text: egui::Color32,
    muted: egui::Color32,
    hover: egui::Color32,
}

struct PlaylistRowInput<'a> {
    ui: &'a mut Ui,
    player: &'a crate::player::Controller,
    index: usize,
    path: &'a std::path::Path,
    colors: RowColors,
}

fn playlist_row(input: PlaylistRowInput<'_>) -> egui::Response {
    let PlaylistRowInput {
        ui,
        player,
        index,
        path,
        colors,
    } = input;
    let title = playlist::file_title(path);
    let duration = player
        .durations
        .get(path)
        .copied()
        .map(format_clock)
        .unwrap_or_else(|| "--:--".into());
    let playing = !player.radio() && player.file_index == Some(index);
    let (row, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    let fill = if player.selected.contains(&index) {
        Some(colors.selection)
    } else if playing {
        Some(colors.accent.gamma_multiply(0.12))
    } else if response.hovered() {
        Some(colors.hover)
    } else {
        None
    };
    if let Some(fill) = fill {
        ui.painter()
            .rect_filled(row, appearance::RADIUS_SMALL, fill);
    }
    let (color, dim) = if playing {
        (colors.accent, colors.accent)
    } else {
        (colors.text, colors.muted)
    };
    let small = egui::FontId::monospace(appearance::FONT_SMALL);
    let middle = row.center().y;
    // The number column; the playing track shows the audio glyph there.
    let number = Rect::from_min_max(row.min, pos2(row.min.x + 32.0, row.max.y));
    if playing {
        appearance::paint_centered_icon(appearance::PaintCenteredIconInput {
            ui,
            rect: number,
            icon: "AudioLines",
            size: appearance::TOOLBAR_ICON,
            tint: colors.accent,
        });
    } else {
        ui.painter().text(
            pos2(number.max.x - 8.0, middle),
            egui::Align2::RIGHT_CENTER,
            index.saturating_add(1),
            small.clone(),
            dim,
        );
    }
    let time = ui.painter().text(
        pos2(row.max.x - 8.0, middle),
        egui::Align2::RIGHT_CENTER,
        duration,
        small,
        dim,
    );
    // A long title stops before the duration.
    let clip = Rect::from_min_max(
        pos2(number.max.x + 4.0, row.min.y),
        pos2(time.min.x - 8.0, row.max.y),
    );
    ui.painter()
        .with_clip_rect(clip.intersect(ui.clip_rect()))
        .text(
            clip.left_center(),
            egui::Align2::LEFT_CENTER,
            title,
            egui::FontId::proportional(appearance::FONT_BODY),
            color,
        );
    response
}

pub(super) fn playlist_footer(app: &mut App, ui: &mut Ui, project: &str) {
    if app.player.url_prompt {
        url_row(app, ui, project);
    }
    if app.player.stack == Some(Stack::List) {
        ui.add(
            appearance::singleline(&mut app.player.playlist_draft)
                .hint_text("Name")
                .desired_width(160.0)
                .id(egui::Id::unique("player-playlist-name")),
        );
    }
    ui.horizontal(|ui| stack_buttons(app, ui));
    stack_menu(app, ui, project);
}

pub(super) fn url_row(app: &mut App, ui: &mut Ui, project: &str) {
    ui.horizontal(|ui| {
        ui.add(
            appearance::singleline(&mut app.player.station_draft)
                .hint_text("https://host/stream")
                .desired_width(260.0)
                .id(egui::Id::unique("player-station-url")),
        );
        if ui.button("Add").clicked() {
            app.add_custom_station(project);
        }
    });
}

pub(super) fn stack_buttons(app: &mut App, ui: &mut Ui) {
    let stacks = [
        (Stack::Add, "Plus", "Add"),
        (Stack::Rem, "Minus", "Remove"),
        (Stack::Sel, "TextSelect", "Select"),
        (Stack::Misc, "ArrowDownWideNarrow", "Misc"),
        (Stack::List, "ListMusic", "List"),
    ];
    for (stack, icon, label) in stacks {
        let lit = app.player.stack == Some(stack);
        let response = footer_button(ui, (icon, label), lit);
        if stack == Stack::Add {
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "player-add-files", response.rect);
        }
        if response.clicked() {
            app.player.stack = if lit { None } else { Some(stack) };
            app.player.confirm = None;
            if stack != Stack::Add {
                app.player.url_prompt = false;
            }
        }
    }
}

pub(super) fn stack_menu(app: &mut App, ui: &mut Ui, project: &str) {
    let Some(stack) = app.player.stack else {
        return;
    };
    ui.add_space(4.0);
    match stack {
        Stack::Add => add_menu(app, ui),
        Stack::Rem => rem_menu(app, ui),
        Stack::Sel => sel_menu(app, ui),
        Stack::Misc => misc_menu(app, ui),
        Stack::List => list_menu(app, ui),
        Stack::Stations => stations_menu(app, ui, project),
    }
}

pub(super) fn add_menu(app: &mut App, ui: &mut Ui) {
    ui.horizontal_wrapped(|ui| {
        if ui.button("File").clicked() {
            app.request_pick(PickKind::AudioFiles);
            app.player.stack = None;
        }
        if ui.button("Directory").clicked() {
            app.request_pick(PickKind::AudioDir);
            app.player.stack = None;
        }
        if ui.button("URL").clicked() {
            app.player.url_prompt = true;
            app.player.stack = None;
        }
    });
}

pub(super) fn rem_menu(app: &mut App, ui: &mut Ui) {
    ui.horizontal_wrapped(|ui| {
        if ui.button("Selected").clicked() {
            app.player_rem_selected();
            app.player.stack = None;
        }
        if ui.button("Crop").clicked() {
            app.player.confirm = Some(PlaylistConfirm::Crop);
        }
        if ui.button("Clear").clicked() {
            app.player.confirm = Some(PlaylistConfirm::Clear);
        }
    });
    confirm_row(app, ui);
}

/// Confirmation for a destructive playlist edit, shown under its menu.
fn confirm_row(app: &mut App, ui: &mut Ui) {
    let Some(confirm) = app.player.confirm else {
        return;
    };
    ui.horizontal_wrapped(|ui| {
        let (question, action) = match confirm {
            PlaylistConfirm::Crop => ("Remove every unselected track?", "Remove"),
            PlaylistConfirm::Clear => ("Remove every track from this playlist?", "Remove"),
            PlaylistConfirm::DeleteList => ("Delete this playlist?", "Delete"),
        };
        ui.label(question);
        if ui.button(action).clicked() {
            match confirm {
                PlaylistConfirm::Crop => app.player_crop(),
                PlaylistConfirm::Clear => app.player_clear_tracks(),
                PlaylistConfirm::DeleteList => app.player_delete_list(),
            }
            app.player.confirm = None;
            app.player.stack = None;
        }
        if ui.button("Cancel").clicked() {
            app.player.confirm = None;
        }
    });
}

pub(super) fn sel_menu(app: &mut App, ui: &mut Ui) {
    ui.horizontal_wrapped(|ui| {
        if ui.button("All").clicked() {
            app.player_sel_all();
            app.player.stack = None;
        }
        if ui.button("None").clicked() {
            app.player_sel_none();
            app.player.stack = None;
        }
        if ui.button("Invert").clicked() {
            app.player_sel_invert();
            app.player.stack = None;
        }
    });
}

pub(super) fn misc_menu(app: &mut App, ui: &mut Ui) {
    ui.horizontal_wrapped(|ui| {
        if ui.button("Sort by title").clicked() {
            app.player_sort_title();
            app.player.stack = None;
        }
        if ui.button("Reverse").clicked() {
            app.player_reverse();
            app.player.stack = None;
        }
        if ui.button("Randomize").clicked() {
            app.player_randomize();
            app.player.stack = None;
        }
        if ui.button("Stations").clicked() {
            app.player.stack = Some(Stack::Stations);
        }
    });
}

pub(super) fn list_menu(app: &mut App, ui: &mut Ui) {
    ui.horizontal_wrapped(|ui| {
        if ui.button("New").clicked() {
            app.player_new_list();
        }
        if ui.button("Rename").clicked() {
            app.player_rename_list();
        }
        if ui.button("Delete").clicked() {
            app.player.confirm = Some(PlaylistConfirm::DeleteList);
        }
    });
    let names: Vec<String> = app
        .preferences
        .playlists
        .iter()
        .map(|playlist| playlist.name.clone())
        .collect();
    let selected = app.preferences.selected_playlist.clone();
    ui.horizontal_wrapped(|ui| {
        for name in names {
            let lit = name == selected;
            if ui.selectable_label(lit, &name).clicked() {
                app.player.confirm = None;
                app.player_switch_list(name);
            }
        }
    });
    confirm_row(app, ui);
}

pub(super) fn stations_menu(app: &mut App, ui: &mut Ui, _project: &str) {
    if ui.button("Open radio catalog").clicked() {
        app.player.radio_mode = true;
        app.preferences.player_radio_mode = true;
        app.player.pl_open = true;
        app.player.stack = None;
    }
}
