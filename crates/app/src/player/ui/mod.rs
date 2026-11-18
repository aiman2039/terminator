//! Flat Terminator-themed player. One center-pane singleton.
use super::app_player::PlayerIconButtonInput;
use super::eq::{EqPreset, eq_bass, eq_flat, eq_treble, format_clock};
use super::*;
use eframe::egui::{self, Color32, Rect, Sense, Stroke, Ui, pos2, vec2};
use engine::Status;

/// Side of the now-playing cover tile; also the height of its row.
const COVER: f32 = 64.0;
/// Diameter of the round Play/Pause button; also the transport row height.
const PLAY_BUTTON: f32 = 40.0;
/// Previous, Play/Pause and Next with the gaps between them.
const TRANSPORT_GAP: f32 = 12.0;
const TRANSPORT_W: f32 = 32.0 + PLAY_BUTTON + 32.0 + 2.0 * TRANSPORT_GAP;

pub(super) fn center(app: &mut App, ui: &mut Ui) {
    ui.set_min_size(ui.available_size());
    ui.horizontal(|ui| {
        ui.strong("Player");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if appearance::sidebar_action(ui, "X", "Close").clicked() {
                app.hide_center_overlay();
            }
            ui.add_space(8.0);
            mode_toggle(app, ui);
        });
    });
    ui.add_space(12.0);
    #[cfg(feature = "test-support")]
    diagnostics::record(ui.ctx(), "player-window", ui.max_rect());
    draw(app, ui);
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
        ui.add_space(12.0);
        surface_panel(app, ui, |app, ui| {
            if app.player.radio_mode {
                radio_panel(app, ui, &project);
            } else {
                playlist_panel(app, ui, &project);
            }
        });
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
    now_playing(app, ui, &status);
    ui.add_space(12.0);
    seek_row(app, ui, &status);
    ui.add_space(6.0);
    transport_row(app, ui, owns && app.player.playing());
}

/// Cover tile, title with its secondary line, and the spectrum.
fn now_playing(app: &App, ui: &mut Ui, status: &Status) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), COVER), Sense::hover());
    let accent = ui.visuals().selection.stroke.color;
    let cover = Rect::from_min_size(rect.min, egui::Vec2::splat(COVER));
    ui.painter().rect_filled(
        cover,
        appearance::RADIUS_LARGE,
        ui.visuals().extreme_bg_color,
    );
    appearance::paint_centered_icon(appearance::PaintCenteredIconInput {
        ui,
        rect: cover,
        icon: if app.player.radio_mode {
            "Radio"
        } else {
            "FileMusic"
        },
        size: COVER * 0.44,
        tint: accent,
    });
    let spectrum_w = (rect.width() * 0.34).min(232.0);
    let spectrum = Rect::from_min_max(pos2(rect.max.x - spectrum_w, rect.min.y), rect.max);
    paint_spectrum(ui, spectrum, &app.player.spectrum);
    let text = Rect::from_min_max(
        pos2(cover.max.x + 16.0, rect.min.y),
        pos2(spectrum.min.x - 16.0, rect.max.y),
    );
    let (title, color) = title_of(ui, status);
    let title = egui::RichText::new(title)
        .strong()
        .color(color)
        .size(appearance::FONT_HEADING);
    let middle = text.center().y;
    text_line(ui, text.with_max_y(middle - 1.0), egui::Align::Max, title);
    let secondary = egui::RichText::new(subtitle(app, status))
        .weak()
        .size(appearance::FONT_SMALL);
    text_line(
        ui,
        text.with_min_y(middle + 1.0),
        egui::Align::Min,
        secondary,
    );
}

/// One truncated line in `rect`, on its top (`Min`) or bottom (`Max`) edge.
fn text_line(ui: &mut Ui, rect: Rect, align: egui::Align, text: egui::RichText) {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(align)),
    );
    child.add(egui::Label::new(text).truncate());
}

fn title_of<'a>(ui: &Ui, status: &'a Status) -> (&'a str, Color32) {
    match status {
        Status::Playing { title, .. } | Status::Paused { title, .. } => {
            (title.as_str(), ui.visuals().strong_text_color())
        }
        Status::Loading { title }
        | Status::Buffering { title }
        | Status::Reconnecting { title, .. } => (title.as_str(), ui.visuals().weak_text_color()),
        Status::Error(error) => (error.as_str(), ui.visuals().error_fg_color),
        Status::Stopped => ("Not playing", ui.visuals().weak_text_color()),
    }
}

/// The line under the title: the stream state, else the playlist with the
/// track position.
fn subtitle(app: &App, status: &Status) -> String {
    let active = match status {
        Status::Loading { .. } => return "Loading…".into(),
        Status::Buffering { .. } => return "Buffering…".into(),
        Status::Reconnecting { .. } => return "Reconnecting…".into(),
        Status::Playing { .. } | Status::Paused { .. } => true,
        Status::Stopped | Status::Error(_) => false,
    };
    let radio = if active {
        app.player.radio()
    } else {
        app.player.radio_mode
    };
    if radio {
        return "Radio".into();
    }
    let name = match app.preferences.selected_playlist.as_str() {
        "" => "Playlist",
        name => name,
    };
    let count = app.preferences.selected_tracks().len();
    match app.player.file_index.filter(|_| active) {
        Some(index) => format!("{name} · Track {} of {count}", index.saturating_add(1)),
        None => format!("{name} · {}", track_count(count)),
    }
}

fn paint_spectrum(ui: &Ui, rect: Rect, spectrum: &[f32]) {
    let accent = ui.visuals().selection.stroke.color;
    let gap = 3.0;
    let bars = f32::from(u16::try_from(spectrum.len().max(1)).unwrap_or(u16::MAX));
    let bar_w = ((rect.width() + gap) / bars - gap).max(1.5);
    for (i, value) in spectrum.iter().enumerate() {
        let level = value.clamp(0.06, 1.0);
        let i_f = f32::from(u16::try_from(i).unwrap_or(u16::MAX));
        let x = rect.min.x + i_f * (bar_w + gap);
        let bar = Rect::from_min_max(
            pos2(x, rect.max.y - rect.height() * level),
            pos2(x + bar_w, rect.max.y),
        );
        ui.painter().rect_filled(
            bar,
            appearance::RADIUS_XSMALL,
            accent.gamma_multiply(0.45 + 0.55 * level),
        );
    }
}

fn seek_values(status: &Status) -> (Duration, Option<Duration>, bool) {
    match status {
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
    }
}

/// Elapsed time, the seek bar and the total on one row.
fn seek_row(app: &mut App, ui: &mut Ui, status: &Status) {
    let (position, duration, seekable) = seek_values(status);
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
    let font = egui::FontId::monospace(appearance::FONT_SMALL);
    let painter = ui.painter().clone();
    let left = painter.layout_no_wrap(clock, font.clone(), ui.visuals().text_color());
    let right = painter.layout_no_wrap(total, font, ui.visuals().weak_text_color());
    let (left_size, right_size) = (left.size(), right.size());
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
    let time_rect = Rect::from_min_size(row.min, vec2(left_size.x, row.height()));
    let middle = row.center().y;
    painter.galley(
        pos2(row.min.x, middle - left_size.y / 2.0),
        left,
        Color32::PLACEHOLDER,
    );
    painter.galley(
        pos2(row.max.x - right_size.x, middle - right_size.y / 2.0),
        right,
        Color32::PLACEHOLDER,
    );
    if ui
        .interact(
            time_rect,
            ui.scope_id().with("player-clock"),
            Sense::click(),
        )
        .clicked()
    {
        app.player.remaining_time = !app.player.remaining_time;
    }
    let rect = Rect::from_min_max(
        pos2(time_rect.max.x + 10.0, row.min.y),
        pos2(row.max.x - right_size.x - 10.0, row.max.y),
    );
    let seek = ui.scope_id().with("player-seek");
    let response = ui.interact(rect, seek, Sense::click_and_drag());
    paint_seek_bar(PaintSeekBarInput {
        ui,
        rect,
        fraction,
        seekable,
    });
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

struct PaintSeekBarInput<'a> {
    ui: &'a Ui,
    rect: Rect,
    fraction: f32,
    seekable: bool,
}

fn paint_seek_bar(input: PaintSeekBarInput<'_>) {
    let PaintSeekBarInput {
        ui,
        rect,
        fraction,
        seekable,
    } = input;
    let visuals = ui.visuals();
    let track = Rect::from_center_size(rect.center(), vec2(rect.width(), 4.0));
    ui.painter()
        .rect_filled(track, appearance::RADIUS_XSMALL, visuals.extreme_bg_color);
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
    ui.painter()
        .rect_filled(filled, appearance::RADIUS_XSMALL, fill);
    if seekable {
        let x = track.min.x + track.width() * fraction.clamp(0.0, 1.0);
        ui.painter()
            .circle_filled(pos2(x, track.center().y), 6.0, appearance::icon_color());
    }
}

/// Shuffle and Repeat on the left, the transport in the center, and the
/// volume with the Equalizer on the right.
fn transport_row(app: &mut App, ui: &mut Ui, playing: bool) {
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), PLAY_BUTTON), Sense::hover());
    let zone = |rect: Rect, layout: egui::Layout, salt: &str| {
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(layout)
            .id_salt(salt)
    };
    let to_right = egui::Layout::left_to_right(egui::Align::Center);
    let to_left = egui::Layout::right_to_left(egui::Align::Center);
    let mut left = ui.new_child(zone(row, to_right, "player-modes"));
    mode_buttons(app, &mut left);
    let middle = Rect::from_center_size(row.center(), vec2(TRANSPORT_W, row.height()));
    let mut center = ui.new_child(zone(middle, to_right, "player-transport"));
    center.spacing_mut().item_spacing.x = TRANSPORT_GAP;
    if App::player_tool_button(&mut center, "SkipBack", "Previous").clicked() {
        app.player_skip(-1);
    }
    if play_button(&mut center, playing).clicked() {
        app.player_play_pause();
    }
    if App::player_tool_button(&mut center, "SkipForward", "Next").clicked() {
        app.player_skip(1);
    }
    let mut right = ui.new_child(zone(row, to_left, "player-volume"));
    volume_controls(app, &mut right);
}

fn mode_buttons(app: &mut App, ui: &mut Ui) {
    if App::player_icon_button(PlayerIconButtonInput {
        ui,
        name: "Shuffle",
        tip: "Shuffle",
        lit: app.preferences.player_shuffle,
    })
    .clicked()
    {
        app.preferences.player_shuffle = !app.preferences.player_shuffle;
        app.player.reset_shuffle();
    }
    if App::player_icon_button(PlayerIconButtonInput {
        ui,
        name: "Repeat",
        tip: "Repeat",
        lit: app.preferences.player_repeat,
    })
    .clicked()
    {
        app.preferences.player_repeat = !app.preferences.player_repeat;
    }
}

/// Laid out right to left: Equalizer, the volume slider, then its icon.
fn volume_controls(app: &mut App, ui: &mut Ui) {
    if App::player_icon_button(PlayerIconButtonInput {
        ui,
        name: "FileSliders",
        tip: "Equalizer",
        lit: app.player.eq_open,
    })
    .clicked()
    {
        app.player.eq_open = !app.player.eq_open;
    }
    let mut volume = app.preferences.player_volume;
    ui.spacing_mut().slider_width = 96.0;
    if appearance::slider(
        ui,
        egui::Slider::new(&mut volume, 0.0..=1.0).show_value(false),
    )
    .on_hover_text("Volume")
    .changed()
    {
        app.preferences.player_volume = volume;
        app.player.set_volume(volume);
    }
    ui.add(
        egui::Image::new(icons::source("Volume2"))
            .tint(ui.visuals().weak_text_color())
            .fit_to_exact_size(egui::Vec2::splat(appearance::TOOLBAR_ICON)),
    );
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
    let n = f32::from(u16::try_from(labels.len()).unwrap_or(u16::MAX));
    let slot = rect.width() / n;
    let accent = ui.visuals().selection.stroke.color;
    let groove = ui.visuals().extreme_bg_color;
    let muted = ui.visuals().weak_text_color();
    for (i, label) in labels.iter().enumerate() {
        let i_f = f32::from(u16::try_from(i).unwrap_or(u16::MAX));
        let x = rect.min.x + i_f * slot + slot / 2.0;
        let groove_rect = Rect::from_center_size(pos2(x, rect.min.y + 36.0), vec2(6.0, 64.0));
        let value = if i == 0 {
            app.player.eq_preamp
        } else {
            let Some(value) = app.player.eq_bands.get(i.saturating_sub(1)).copied() else {
                continue;
            };
            value
        };
        if let Some(next) = v_slider(VSliderInput {
            ui,
            rect: groove_rect,
            id: &format!("eq-{i}"),
            value,
            groove,
            thumb: accent,
        }) {
            if i == 0 {
                app.player.eq_preamp = next;
            } else if let Some(slot) = app.player.eq_bands.get_mut(i.saturating_sub(1)) {
                *slot = next;
            }
        }
        ui.painter().text(
            pos2(x, rect.max.y - 2.0),
            egui::Align2::CENTER_BOTTOM,
            *label,
            egui::FontId::monospace(appearance::FONT_MINI),
            muted,
        );
    }
}

mod playlist_view;
mod radio_view;
mod widgets;
use playlist_view::*;
use radio_view::*;
use widgets::*;

#[cfg(test)]
mod tests;
