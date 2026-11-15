//! Paint of the change rows: stage boxes, counts and status badges.
use super::*;

fn painted(ctx: &egui::Context, add: impl FnMut(&mut egui::Ui)) -> Vec<egui::epaint::Shape> {
    let mut add = add;
    // The second frame has laid out every widget.
    ctx.run_ui(egui::RawInput::default(), &mut add)
        .textures_delta
        .clear();
    let mut output = ctx.run_ui(egui::RawInput::default(), &mut add);
    output.textures_delta.clear();
    output
        .shapes
        .into_iter()
        .map(|clipped| clipped.shape)
        .collect()
}

fn rect_strokes(shapes: &[egui::epaint::Shape]) -> Vec<Color32> {
    shapes
        .iter()
        .filter_map(|shape| match shape {
            egui::epaint::Shape::Rect(rect) => Some(rect.stroke.color),
            _ => None,
        })
        .filter(|color| *color != Color32::TRANSPARENT)
        .collect()
}

/// The stage box is custom paint; at rest it takes the same outline as an
/// egui checkbox, not the faint theme border.
#[test]
fn an_unchecked_stage_box_is_outlined_like_a_checkbox_at_rest() {
    let ctx = egui::Context::default();
    crate::appearance::install(&ctx);
    let shapes = painted(&ctx, |ui| {
        ui.checkbox(&mut false, "Amend");
        tri_checkbox(ui, CheckState::Unchecked, None);
    });
    let strokes = rect_strokes(&shapes);
    let [checkbox, stage_box] = strokes[..] else {
        panic!("checkbox and stage box: {strokes:?}");
    };
    assert_eq!(stage_box, checkbox, "{strokes:?}");
    let border = appearance::color(&AppearanceConfig::default().border);
    assert_ne!(stage_box, border);
}

#[test]
fn the_trailer_colors_counts_and_puts_the_letter_on_a_badge() {
    let theme = AppearanceConfig::default();
    let added = appearance::color(&theme.git_added);
    let deleted = appearance::color(&theme.git_deleted);
    assert_eq!(
        change_trailer(&theme, Some((3, 2)), 'M'),
        vec![
            TrailPart::Count("+3".into(), added),
            TrailPart::Count("−2".into(), deleted),
            TrailPart::Badge('M', git_color(&theme, 'M')),
        ]
    );
    // No deleted lines: no `−0`. No diffstat: the badge alone.
    assert_eq!(
        change_trailer(&theme, Some((1, 0)), 'U'),
        vec![
            TrailPart::Count("+1".into(), added),
            TrailPart::Badge('U', git_color(&theme, 'U')),
        ]
    );
    assert_eq!(
        change_trailer(&theme, None, 'D'),
        vec![TrailPart::Badge('D', git_color(&theme, 'D'))]
    );
}

/// A painted row: each count in its own color, and the letter over a
/// filled badge of its status color.
#[test]
fn a_change_row_paints_each_trailer_part_in_its_color() {
    let ctx = egui::Context::default();
    crate::appearance::install(&ctx);
    let theme = AppearanceConfig::default();
    let trailer = change_trailer(&theme, Some((3, 2)), 'M');
    let shapes = painted(&ctx, |ui| {
        ui.set_width(300.0);
        name_row(NameRowInput {
            ui,
            icon: icons::file_icon(Path::new("app.ts")),
            icon_tint: appearance::icon_color(),
            name: "app.ts",
            color: git_color(&theme, 'M'),
            hint: "",
            trailing: &trailer,
        });
    });
    let text_color = |wanted: &str| {
        shapes.iter().find_map(|shape| match shape {
            egui::epaint::Shape::Text(text) if text.galley.text() == wanted => {
                text.galley.job.sections.first().map(|s| s.format.color)
            }
            _ => None,
        })
    };
    let modified = git_color(&theme, 'M');
    assert_eq!(text_color("+3"), Some(appearance::color(&theme.git_added)));
    assert_eq!(
        text_color("−2"),
        Some(appearance::color(&theme.git_deleted))
    );
    assert_eq!(text_color("M"), Some(modified));
    let badge = shapes.iter().any(|shape| {
        matches!(shape, egui::epaint::Shape::Rect(rect)
            if rect.fill == modified.gamma_multiply(BADGE_TINT))
    });
    assert!(badge, "a tinted badge under the letter");
}

/// The line above the panel names the repository; "Last known
/// directory" stays only while the session has not confirmed it.
#[test]
fn the_repository_line_names_the_repository_and_keeps_the_unconfirmed_note() {
    let texts = |folder: Option<&Path>, unconfirmed: bool| -> Vec<String> {
        let ctx = egui::Context::default();
        crate::appearance::install(&ctx);
        let shapes = painted(&ctx, |ui| {
            ui.set_width(300.0);
            repository_line(ui, folder, unconfirmed);
        });
        shapes
            .iter()
            .filter_map(|shape| match shape {
                egui::epaint::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect()
    };
    let repo = Path::new("/work/atlas-web");
    assert_eq!(texts(Some(repo), false), vec!["atlas-web".to_owned()]);
    let unconfirmed = texts(Some(repo), true);
    assert!(
        unconfirmed.contains(&"atlas-web".to_owned())
            && unconfirmed.contains(&"Last known directory".to_owned()),
        "{unconfirmed:?}"
    );
    assert_eq!(texts(None, true), vec!["Last known directory".to_owned()]);
    assert!(texts(None, false).is_empty());
    assert_eq!(repository_line::repository_name(Path::new("/")), "/");
}
