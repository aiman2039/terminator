use super::*;

const STATION_ROW_H: f32 = 22.0;

pub(super) fn radio_panel(app: &mut App, ui: &mut Ui, project: &str) {
    let width = ui.available_width();
    ui.set_max_width(width);
    ui.strong("Radio");
    ui.add(
        appearance::singleline(&mut app.player.radio_query)
            .hint_text("Search name, category, country…")
            .desired_width(f32::INFINITY)
            .lock_focus(true),
    );
    category_combo(app, ui, width);
    ui.add_space(4.0);
    let stations = app.radio_visible();
    let playing_url = if app.player.radio() {
        app.radio_listing()
            .get(app.player.station_index.unwrap_or(0))
            .map(|station| station.url.clone())
    } else {
        None
    };
    let height = (ui.available_height() - 72.0).max(120.0);
    if stations.is_empty() {
        ui.weak("No stations match.");
    }
    let play = station_list(
        ui,
        StationListInput {
            stations: &stations,
            playing_url: playing_url.as_deref(),
            width,
            height,
        },
    );
    ui.add_space(6.0);
    custom_station_row(app, ui, project);
    ui.weak(format!("{} stations", stations.len()));
    if let Some(station) = play {
        app.play_radio(project, station);
    }
}

fn category_combo(app: &mut App, ui: &mut Ui, width: f32) {
    let categories = radio::categories_ready();
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
}

struct StationListInput<'a> {
    stations: &'a [radio::Station],
    playing_url: Option<&'a str>,
    width: f32,
    height: f32,
}

/// The scrolling station list; returns the clicked station.
fn station_list(ui: &mut Ui, input: StationListInput<'_>) -> Option<radio::Station> {
    let StationListInput {
        stations,
        playing_url,
        width,
        height,
    } = input;
    let mut play = None;
    egui::ScrollArea::vertical()
        .id_salt("player-radio")
        .max_height(height)
        .max_width(width)
        .auto_shrink([false, false])
        .show_rows(ui, STATION_ROW_H, stations.len(), |ui, range| {
            ui.set_max_width(width);
            for index in range {
                let Some(station) = stations.get(index) else {
                    continue;
                };
                let lit = playing_url == Some(station.url.as_str());
                if station_row(ui, station, lit) {
                    play = Some(station.clone());
                }
            }
        });
    play
}

/// One painted station row; returns whether it was clicked.
fn station_row(ui: &mut Ui, station: &radio::Station, lit: bool) -> bool {
    let (row, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), STATION_ROW_H), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if lit {
        ui.painter()
            .rect_filled(row, 3.0, ui.visuals().selection.bg_fill);
    } else if response.hovered() {
        ui.painter()
            .rect_filled(row, 3.0, ui.visuals().widgets.hovered.bg_fill);
    }
    let icon = Rect::from_center_size(pos2(row.min.x + 12.0, row.center().y), vec2(16.0, 16.0));
    egui::Image::new(icons::source("Radio"))
        .tint(appearance::icon_color())
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
    response.clicked()
}

fn custom_station_row(app: &mut App, ui: &mut Ui, project: &str) {
    ui.horizontal(|ui| {
        ui.add(
            appearance::singleline(&mut app.player.radio_draft_name)
                .hint_text("Name")
                .desired_width(100.0)
                .id(egui::Id::unique("player-radio-name")),
        );
        ui.add(
            appearance::singleline(&mut app.player.station_draft)
                .hint_text("https://host/stream")
                .desired_width(180.0)
                .id(egui::Id::unique("player-station-url")),
        );
        if ui.button("Add station").clicked() {
            app.add_custom_station(project);
        }
    });
}
