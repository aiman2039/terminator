//! Flat Terminator-themed player. One floating window.
use super::*;
use eframe::egui::{self, Color32, Rect, Sense, Stroke, Ui, pos2, vec2};
use engine::Status;

pub(super) fn window(app: &mut App, ctx: &egui::Context) {
    let mut open = true;
    app.popups
        .window(ctx, "Player")
        .id(egui::Id::new("terminator-player"))
        .collapsible(false)
        .default_size([440.0, 560.0])
        .min_size([360.0, 220.0])
        .resizable([true, true])
        .open(&mut open)
        .show(ctx, |ui| {
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "player-window", ui.max_rect());
            draw(app, ui);
        });
    app.player_open &= open;
}

fn draw(app: &mut App, ui: &mut Ui) {
    let project = app.player_owner().unwrap_or_default();
    app.player.set_volume(app.preferences.player_volume);
    let playing = matches!(app.player.status, Status::Playing { .. });
    if playing {
        ui.ctx().request_repaint_after(Duration::from_millis(80));
    }
    main_panel(app, ui, &project);
    if app.player.eq_open {
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(6.0);
        eq_panel(app, ui);
    }
    if app.player.pl_open {
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(6.0);
        if app.player.radio_mode {
            radio_panel(app, ui, &project);
        } else {
            playlist_panel(app, ui, &project);
        }
    }
    space_toggle(app, ui, &project);
}

fn space_toggle(app: &mut App, ui: &mut Ui, project: &str) {
    let owns = app.player.project.as_deref() == Some(project);
    if owns
        && !ui.ctx().egui_wants_keyboard_input()
        && ui.input(|i| i.key_pressed(egui::Key::Space))
    {
        ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space));
        app.player.toggle();
    }
}

fn main_panel(app: &mut App, ui: &mut Ui, project: &str) {
    let owns = app.player.project.as_deref() == Some(project);
    let status = if owns {
        app.player.status.clone()
    } else {
        Status::Stopped
    };
    title_row(ui, &status);
    seek_row(app, ui, &status);
    vis_row(ui, &app.player.spectrum);
    ui.add_space(6.0);
    volume_row(app, ui);
    ui.add_space(4.0);
    controls_row(app, ui);
}

fn title_row(ui: &mut Ui, status: &Status) {
    let (title, color) = match status {
        Status::Playing { title, .. } | Status::Paused { title, .. } => {
            (title.as_str(), ui.visuals().text_color())
        }
        Status::Error(error) => (error.as_str(), ui.visuals().error_fg_color),
        Status::Stopped => ("Not playing", ui.visuals().weak_text_color()),
    };
    ui.add(
        egui::Label::new(egui::RichText::new(title).strong().color(color).size(14.0)).truncate(),
    );
}

fn seek_row(app: &mut App, ui: &mut Ui, status: &Status) {
    let (position, duration, seekable) = match status {
        Status::Playing {
            position,
            duration,
            seekable,
            ..
        }
        | Status::Paused {
            position,
            duration,
            seekable,
            ..
        } => (*position, *duration, *seekable),
        _ => (Duration::ZERO, None, false),
    };
    let fraction = match duration {
        Some(total) if !total.is_zero() => {
            (position.as_secs_f32() / total.as_secs_f32()).clamp(0.0, 1.0)
        }
        _ => 0.0,
    };
    let clock = if app.player.remaining_time
        && let Some(total) = duration
    {
        format!("-{}", format_clock(total.saturating_sub(position)))
    } else {
        format_clock(position)
    };
    let total = if seekable {
        duration.map(format_clock).unwrap_or_else(|| "--:--".into())
    } else {
        "Live".into()
    };
    ui.horizontal(|ui| {
        let time =
            ui.add(egui::Label::new(egui::RichText::new(clock).monospace()).sense(Sense::click()));
        if time.clicked() {
            app.player.remaining_time = !app.player.remaining_time;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.weak(egui::RichText::new(total).monospace());
        });
    });
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click_and_drag());
    paint_seek_bar(ui, rect, fraction, seekable);
    if seekable
        && (response.clicked() || response.dragged())
        && let Some(total) = duration
        && let Some(pos) = response.interact_pointer_pos()
    {
        let t = ((pos.x - rect.min.x) / rect.width().max(1.0)).clamp(0.0, 1.0);
        app.player.seek_fraction(t, total);
    }
    if !seekable {
        response.on_hover_text("Live stream — seeking is not available");
    }
}

fn paint_seek_bar(ui: &Ui, rect: Rect, fraction: f32, seekable: bool) {
    let visuals = ui.visuals();
    let track = Rect::from_center_size(rect.center(), vec2(rect.width(), 8.0));
    ui.painter()
        .rect_filled(track, 4.0, visuals.extreme_bg_color);
    let filled_w = if seekable {
        track.width() * fraction.clamp(0.0, 1.0)
    } else {
        track.width()
    };
    let filled = track.with_max_x(track.min.x + filled_w);
    let fill = if seekable {
        visuals.selection.stroke.color
    } else {
        visuals.weak_text_color().gamma_multiply(0.45)
    };
    ui.painter().rect_filled(filled, 4.0, fill);
    if seekable {
        let x = track.min.x + track.width() * fraction.clamp(0.0, 1.0);
        ui.painter()
            .circle_filled(pos2(x, track.center().y), 7.0, appearance::ICON_COLOR);
    }
}

fn vis_row(ui: &mut Ui, spectrum: &[f32]) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
    let accent = ui.visuals().selection.stroke.color;
    let bars = spectrum.len().max(1);
    let gap = 2.0;
    let bar_w = ((rect.width() - 4.0) / bars as f32 - gap).max(1.5);
    for (i, value) in spectrum.iter().enumerate() {
        let wave = value.clamp(0.04, 1.0);
        let h = (rect.height() - 4.0) * wave;
        let x = rect.min.x + 2.0 + i as f32 * (bar_w + gap);
        let bar = Rect::from_min_max(
            pos2(x, rect.max.y - 2.0 - h),
            pos2(x + bar_w, rect.max.y - 2.0),
        );
        ui.painter()
            .rect_filled(bar, 1.0, accent.gamma_multiply(0.85));
    }
}

fn volume_row(app: &mut App, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.weak("Vol");
        let mut volume = app.preferences.player_volume;
        ui.spacing_mut().slider_width = ui.available_width();
        if ui
            .add(egui::Slider::new(&mut volume, 0.0..=1.0).show_value(false))
            .changed()
        {
            app.preferences.player_volume = volume;
            app.player.set_volume(volume);
        }
    });
}

fn controls_row(app: &mut App, ui: &mut Ui) {
    ui.horizontal(|ui| {
        if App::player_tool_button(ui, "SkipBack", "Previous").clicked() {
            app.player_skip(-1);
        }
        if App::player_tool_button(ui, "Play", "Play").clicked() {
            app.player_play_button();
        }
        if App::player_tool_button(ui, "Pause", "Pause").clicked() {
            app.player.pause();
        }
        if App::player_tool_button(ui, "Square", "Stop").clicked() && app.player.project.is_some() {
            app.player.stop();
        }
        if App::player_tool_button(ui, "SkipForward", "Next").clicked() {
            app.player_skip(1);
        }
        if App::player_tool_button(ui, "FolderOpen", "Add files").clicked() {
            app.pick_audio = true;
        }
        ui.add_space(8.0);
        if App::player_icon_button(ui, "Shuffle", "Shuffle", app.preferences.player_shuffle)
            .clicked()
        {
            app.preferences.player_shuffle = !app.preferences.player_shuffle;
            app.player.shuffle_bag.clear();
        }
        if App::player_icon_button(ui, "Repeat", "Repeat", app.preferences.player_repeat).clicked()
        {
            app.preferences.player_repeat = !app.preferences.player_repeat;
        }
        ui.add_space(8.0);
        if App::player_icon_button(ui, "FileSliders", "Equalizer", app.player.eq_open).clicked() {
            app.player.eq_open = !app.player.eq_open;
        }
        ui.add_space(8.0);
        mode_toggle(app, ui);
    });
}

fn eq_panel(app: &mut App, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.strong("Equalizer");
        ui.add_space(8.0);
        if chip(ui, "On", app.player.eq_on).clicked() {
            app.player.eq_on = !app.player.eq_on;
        }
        let flat = eq_flat();
        let bass = eq_bass();
        let treble = eq_treble();
        if chip(ui, "Flat", eq_matches(app, &flat)).clicked() {
            app.player_eq_preset(flat);
        }
        if chip(ui, "Bass", eq_matches(app, &bass)).clicked() {
            app.player_eq_preset(bass);
        }
        if chip(ui, "Treble", eq_matches(app, &treble)).clicked() {
            app.player_eq_preset(treble);
        }
    });
    ui.add_space(4.0);
    let labels = [
        "pre", "60", "170", "310", "600", "1k", "3k", "6k", "12k", "14k", "16k",
    ];
    let height = 88.0;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let n = labels.len() as f32;
    let slot = rect.width() / n;
    let accent = ui.visuals().selection.stroke.color;
    let groove = ui.visuals().extreme_bg_color;
    let muted = ui.visuals().weak_text_color();
    for (i, label) in labels.iter().enumerate() {
        let x = rect.min.x + i as f32 * slot + slot / 2.0;
        let groove_rect = Rect::from_center_size(pos2(x, rect.min.y + 36.0), vec2(6.0, 64.0));
        let value = if i == 0 {
            app.player.eq_preamp
        } else {
            app.player.eq_bands[i - 1]
        };
        if let Some(next) = v_slider(ui, groove_rect, &format!("eq-{i}"), value, groove, accent) {
            if i == 0 {
                app.player.eq_preamp = next;
            } else {
                app.player.eq_bands[i - 1] = next;
            }
        }
        ui.painter().text(
            pos2(x, rect.max.y - 2.0),
            egui::Align2::CENTER_BOTTOM,
            *label,
            egui::FontId::monospace(9.0),
            muted,
        );
    }
}

fn playlist_panel(app: &mut App, ui: &mut Ui, project: &str) {
    let extra = app.player.stack.is_some() || app.player.url_prompt;
    let footer_h = if extra { 92.0 } else { 36.0 };
    let height = (ui.available_height() - footer_h).max(120.0);
    let (list, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    playlist_rows(app, ui, list, project);
    playlist_footer(app, ui, project);
}

fn playlist_rows(app: &mut App, ui: &mut Ui, rect: Rect, project: &str) {
    let surface = appearance::color(&app.theme.surface);
    ui.painter().rect_filled(rect, 3.0, surface);
    let tracks = app.playlist();
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink(4.0))
            .layout(egui::Layout::top_down(egui::Align::Min))
            .id_salt("player-rows"),
    );
    if tracks.is_empty() {
        child.weak("No tracks yet.");
        return;
    }
    let mut play = None;
    let mut click = None;
    let selection = child.visuals().selection.bg_fill;
    let accent = child.visuals().selection.stroke.color;
    let text = child.visuals().text_color();
    egui::ScrollArea::vertical()
        .id_salt("player-playlist")
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            for (index, path) in tracks.iter().enumerate() {
                let title = playlist::file_title(path);
                let duration = app
                    .player
                    .durations
                    .get(path)
                    .copied()
                    .map(format_clock)
                    .unwrap_or_else(|| "--:--".into());
                let playing = !app.player.radio() && app.player.file_index == Some(index);
                let selected = app.player.selected.contains(&index);
                let (row, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
                if selected {
                    ui.painter().rect_filled(row, 2.0, selection);
                }
                let color = if playing { accent } else { text };
                ui.painter().text(
                    pos2(row.min.x + 6.0, row.center().y),
                    egui::Align2::LEFT_CENTER,
                    format!("{}. {title}", index + 1),
                    egui::FontId::proportional(13.0),
                    color,
                );
                ui.painter().text(
                    pos2(row.max.x - 6.0, row.center().y),
                    egui::Align2::RIGHT_CENTER,
                    duration,
                    egui::FontId::monospace(12.0),
                    color,
                );
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
    if ui.input(|i| i.key_pressed(egui::Key::Delete)) {
        app.player_rem_selected();
    }
}

fn playlist_footer(app: &mut App, ui: &mut Ui, project: &str) {
    if app.player.url_prompt {
        url_row(app, ui, project);
    }
    if app.player.stack == Some(Stack::List) {
        ui.add(
            egui::TextEdit::singleline(&mut app.player.playlist_draft)
                .hint_text("Name")
                .desired_width(160.0)
                .id(egui::Id::new("player-playlist-name")),
        );
    }
    ui.horizontal(|ui| {
        stack_buttons(app, ui);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.weak(running_time(app));
        });
    });
    stack_menu(app, ui, project);
}

fn url_row(app: &mut App, ui: &mut Ui, project: &str) {
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut app.player.station_draft)
                .hint_text("https://host/stream")
                .desired_width(260.0)
                .id(egui::Id::new("player-station-url")),
        );
        if ui.button("Add").clicked() {
            app.add_custom_station(project);
        }
    });
}

fn running_time(app: &App) -> String {
    let position = match &app.player.status {
        Status::Playing { position, .. } | Status::Paused { position, .. } => *position,
        _ => Duration::ZERO,
    };
    let total: Duration = app
        .player
        .durations
        .values()
        .copied()
        .fold(Duration::ZERO, Duration::saturating_add);
    if total.is_zero() {
        format!("{}/--:--", format_clock(position))
    } else {
        format!("{}/{}", format_clock(position), format_clock(total))
    }
}

fn stack_buttons(app: &mut App, ui: &mut Ui) {
    let stacks = [
        (Stack::Add, "Add"),
        (Stack::Rem, "Rem"),
        (Stack::Sel, "Sel"),
        (Stack::Misc, "Misc"),
        (Stack::List, "List"),
    ];
    for (stack, label) in stacks {
        let lit = app.player.stack == Some(stack);
        let response = chip(ui, label, lit);
        if label == "Add" {
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "player-add-files", response.rect);
        }
        if response.clicked() {
            app.player.stack = if lit { None } else { Some(stack) };
            if stack != Stack::Add {
                app.player.url_prompt = false;
            }
        }
    }
}

fn stack_menu(app: &mut App, ui: &mut Ui, project: &str) {
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

fn add_menu(app: &mut App, ui: &mut Ui) {
    ui.horizontal_wrapped(|ui| {
        if ui.button("File").clicked() {
            app.pick_audio = true;
            app.player.stack = None;
        }
        if ui.button("Directory").clicked() {
            app.pick_audio_dir = true;
            app.player.stack = None;
        }
        if ui.button("URL").clicked() {
            app.player.url_prompt = true;
            app.player.stack = None;
        }
    });
}

fn rem_menu(app: &mut App, ui: &mut Ui) {
    ui.horizontal_wrapped(|ui| {
        if ui.button("Selected").clicked() {
            app.player_rem_selected();
            app.player.stack = None;
        }
        if ui.button("Crop").clicked() {
            app.player_crop();
            app.player.stack = None;
        }
        if ui.button("Clear").clicked() {
            app.player_clear_tracks();
            app.player.stack = None;
        }
    });
}

fn sel_menu(app: &mut App, ui: &mut Ui) {
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

fn misc_menu(app: &mut App, ui: &mut Ui) {
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

fn list_menu(app: &mut App, ui: &mut Ui) {
    ui.horizontal_wrapped(|ui| {
        if ui.button("New").clicked() {
            app.player_new_list();
        }
        if ui.button("Rename").clicked() {
            app.player_rename_list();
        }
        if ui.button("Delete").clicked() {
            app.player_delete_list();
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
                app.player_switch_list(name);
            }
        }
    });
}

fn stations_menu(app: &mut App, ui: &mut Ui, _project: &str) {
    if ui.button("Open radio catalog").clicked() {
        app.player.radio_mode = true;
        app.preferences.player_radio_mode = true;
        app.player.pl_open = true;
        app.player.stack = None;
    }
}

fn radio_panel(app: &mut App, ui: &mut Ui, project: &str) {
    let width = ui.available_width();
    ui.set_max_width(width);
    ui.strong("Radio");
    ui.add(
        egui::TextEdit::singleline(&mut app.player.radio_query)
            .hint_text("Search name, category, country…")
            .desired_width(f32::INFINITY)
            .lock_focus(true),
    );
    let categories = radio::categories();
    let mut category = app.player.radio_category.clone();
    let category_label = if category.is_empty() {
        "All categories".to_string()
    } else {
        category.clone()
    };
    egui::ComboBox::from_id_salt("player-radio-category")
        .selected_text(category_label)
        .width(width)
        .wrap_mode(egui::TextWrapMode::Truncate)
        .show_ui(ui, |ui| {
            ui.set_max_width(width);
            ui.selectable_value(&mut category, String::new(), "All categories");
            for name in categories {
                ui.selectable_value(&mut category, name.clone(), name.as_str());
            }
        });
    app.player.radio_category = category;
    ui.add_space(4.0);
    let stations = app.radio_visible();
    let playing_url = if app.player.radio() {
        app.radio_listing()
            .get(app.player.station_index.unwrap_or(0))
            .map(|station| station.url.clone())
    } else {
        None
    };
    let mut play = None;
    let height = (ui.available_height() - 72.0).max(120.0);
    const ROW_H: f32 = 22.0;
    if stations.is_empty() {
        ui.weak("No stations match.");
    }
    egui::ScrollArea::vertical()
        .id_salt("player-radio")
        .max_height(height)
        .max_width(width)
        .auto_shrink([false, false])
        .show_rows(ui, ROW_H, stations.len(), |ui, range| {
            ui.set_max_width(width);
            for index in range {
                let Some(station) = stations.get(index) else {
                    continue;
                };
                let lit = playing_url.as_deref() == Some(station.url.as_str());
                let (row, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                if lit {
                    ui.painter()
                        .rect_filled(row, 3.0, ui.visuals().selection.bg_fill);
                } else if response.hovered() {
                    ui.painter()
                        .rect_filled(row, 3.0, ui.visuals().widgets.hovered.bg_fill);
                }
                let icon = Rect::from_center_size(
                    pos2(row.min.x + 12.0, row.center().y),
                    vec2(16.0, 16.0),
                );
                egui::Image::new(icons::source("Radio"))
                    .tint(appearance::ICON_COLOR)
                    .paint_at(ui, icon);
                let mut label = station.name.clone();
                if !station.country.is_empty() {
                    label = format!("{label}  {}", station.country);
                }
                let color = if lit {
                    ui.visuals().selection.stroke.color
                } else {
                    ui.visuals().text_color()
                };
                ui.painter().text(
                    pos2(row.min.x + 28.0, row.center().y),
                    egui::Align2::LEFT_CENTER,
                    label,
                    egui::FontId::proportional(13.0),
                    color,
                );
                if response.clicked() {
                    play = Some(station.clone());
                }
            }
        });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut app.player.radio_draft_name)
                .hint_text("Name")
                .desired_width(100.0)
                .id(egui::Id::new("player-radio-name")),
        );
        ui.add(
            egui::TextEdit::singleline(&mut app.player.station_draft)
                .hint_text("https://host/stream")
                .desired_width(180.0)
                .id(egui::Id::new("player-station-url")),
        );
        if ui.button("Add station").clicked() {
            app.add_custom_station(project);
        }
    });
    ui.weak(format!("{} stations", stations.len()));
    if let Some(station) = play {
        app.play_radio(project, station);
    }
}

fn mode_toggle(app: &mut App, ui: &mut Ui) {
    let radio = app.player.radio_mode;
    let (rect, _) = ui.allocate_exact_size(vec2(72.0, 32.0), Sense::hover());
    ui.painter().rect(
        rect,
        8.0,
        ui.visuals().extreme_bg_color,
        ui.visuals().widgets.inactive.bg_stroke,
        egui::StrokeKind::Inside,
    );
    let mid = rect.center().x;
    let files = Rect::from_min_max(rect.min, pos2(mid, rect.max.y)).shrink(2.0);
    let radio_rect = Rect::from_min_max(pos2(mid, rect.min.y), rect.max).shrink(2.0);
    if mode_half(ui, files, "ListMusic", "Files", !radio).clicked() {
        app.player.radio_mode = false;
        app.preferences.player_radio_mode = false;
        app.player.pl_open = true;
    }
    if mode_half(ui, radio_rect, "Radio", "Radio", radio).clicked() {
        app.player.radio_mode = true;
        app.preferences.player_radio_mode = true;
        app.player.pl_open = true;
    }
}

fn mode_half(ui: &mut Ui, rect: Rect, icon: &str, tip: &str, active: bool) -> egui::Response {
    let response = ui
        .interact(rect, ui.id().with(icon), Sense::click())
        .on_hover_text(tip);
    if active {
        ui.painter()
            .rect_filled(rect, 6.0, ui.visuals().selection.bg_fill);
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, 6.0, ui.visuals().widgets.hovered.bg_fill);
    }
    let tint = if active {
        ui.visuals().selection.stroke.color
    } else {
        appearance::ICON_COLOR
    };
    egui::Image::new(icons::source(icon))
        .tint(tint)
        .paint_at(ui, Rect::from_center_size(rect.center(), vec2(16.0, 16.0)));
    response
}

fn chip(ui: &mut Ui, label: &str, active: bool) -> egui::Response {
    let fill = if active {
        ui.visuals().selection.bg_fill
    } else {
        Color32::TRANSPARENT
    };
    let stroke = if active {
        Stroke::new(1.0, ui.visuals().selection.stroke.color)
    } else {
        ui.visuals().widgets.inactive.bg_stroke
    };
    ui.add(
        egui::Button::new(egui::RichText::new(label).size(13.0))
            .fill(fill)
            .stroke(stroke)
            .corner_radius(6.0)
            .min_size(vec2(56.0, 28.0)),
    )
}

fn eq_matches(app: &App, preset: &EqPreset) -> bool {
    let EqPreset { preamp, bands } = *preset;
    (app.player.eq_preamp - preamp).abs() < 0.03
        && app
            .player
            .eq_bands
            .iter()
            .zip(bands)
            .all(|(left, right)| (left - right).abs() < 0.03)
}

fn v_slider(
    ui: &mut Ui,
    rect: Rect,
    id: &str,
    value: f32,
    groove: Color32,
    thumb: Color32,
) -> Option<f32> {
    let response = ui.interact(rect, ui.id().with(id), Sense::click_and_drag());
    ui.painter().rect_filled(rect, 2.0, groove);
    let y = rect.max.y - 4.0 - (rect.height() - 8.0) * value.clamp(0.0, 1.0);
    let knob = Rect::from_center_size(pos2(rect.center().x, y), vec2(rect.width() + 4.0, 6.0));
    ui.painter().rect_filled(knob, 2.0, thumb);
    if response.clicked() || response.dragged() {
        return response.interact_pointer_pos().map(|pos| {
            (1.0 - (pos.y - rect.min.y - 4.0) / (rect.height() - 8.0).max(1.0)).clamp(0.0, 1.0)
        });
    }
    None
}
