//! Commit list painting: lanes, curves, ref pills, clicks, and culling.
use super::commit_graph::{CommitList, LaneGeometry};
use crate::appearance::lane_color;
use crate::egui;
use crate::git_log::{CommitEntry, CommitLog, GraphRow, RefLabel, RefLabelKind, graph_rows};
use terminator_core::appearance::AppearanceConfig;

fn entry(hash: &str, parents: &[&str], refs: &[(&str, RefLabelKind)]) -> CommitEntry {
    CommitEntry {
        hash: hash.into(),
        short_hash: hash.chars().take(7).collect(),
        author: "Fixture".into(),
        email: "fixture@example.invalid".into(),
        date: "1h ago".into(),
        timestamp: 0,
        message: format!("subject {hash}"),
        subject: format!("subject {hash}"),
        refs: refs
            .iter()
            .map(|(name, kind)| RefLabel {
                name: (*name).into(),
                kind: *kind,
            })
            .collect(),
        parents: parents.iter().map(|parent| (*parent).into()).collect(),
        graph: GraphRow::default(),
    }
}

/// A log with lanes laid out the way `fetch_log` lays them out.
fn log(mut commits: Vec<CommitEntry>) -> CommitLog {
    let rows = graph_rows(
        commits
            .iter()
            .map(|c| (c.hash.as_str(), c.parents.as_slice())),
    );
    for (commit, row) in commits.iter_mut().zip(rows) {
        commit.graph = row;
    }
    let lanes = commits.iter().map(|c| c.graph.width()).max().unwrap_or(0);
    CommitLog {
        commits,
        lanes,
        ..Default::default()
    }
}

struct Frame {
    clicked: Option<String>,
    shapes: Vec<egui::Shape>,
}

fn flatten(shape: egui::Shape, out: &mut Vec<egui::Shape>) {
    if let egui::Shape::Vec(shapes) = shape {
        for shape in shapes {
            flatten(shape, out);
        }
    } else {
        out.push(shape);
    }
}

fn frame(
    ctx: &egui::Context,
    log: &CommitLog,
    size: egui::Vec2,
    events: Vec<egui::Event>,
) -> Frame {
    let theme = AppearanceConfig::default();
    let mut clicked = None;
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
        events,
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        clicked = CommitList {
            log,
            theme: &theme,
            selected: None,
            reveal: None,
        }
        .show(ui);
    });
    output.textures_delta.clear();
    let mut shapes = Vec::new();
    for clipped in output.shapes {
        flatten(clipped.shape, &mut shapes);
    }
    Frame { clicked, shapes }
}

fn dots(shapes: &[egui::Shape]) -> Vec<&egui::epaint::CircleShape> {
    shapes
        .iter()
        .filter_map(|shape| match shape {
            egui::Shape::Circle(circle) => Some(circle),
            _ => None,
        })
        .collect()
}

fn texts(shapes: &[egui::Shape]) -> Vec<String> {
    shapes
        .iter()
        .filter_map(|shape| match shape {
            egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_long_log_paints_only_the_visible_rows() {
    let commits: Vec<CommitEntry> = (0..1000_usize)
        .map(|index| {
            let parent = format!("c{}", index.saturating_add(1));
            entry(&format!("c{index}"), &[parent.as_str()], &[])
        })
        .collect();
    let log = log(commits);
    let painted = frame(
        &egui::Context::default(),
        &log,
        egui::vec2(400.0, 300.0),
        vec![],
    );
    let dots = dots(&painted.shapes).len();
    assert!((3..20).contains(&dots), "{dots} rows painted for 300 px");
}

#[test]
fn merge_rows_paint_lane_curves_colored_dots_and_ref_pills() {
    use RefLabelKind::{Head, LocalBranch, RemoteBranch, Tag};
    let log = log(vec![
        entry(
            "m",
            &["c", "b"],
            &[
                ("HEAD", Head),
                ("main", LocalBranch),
                ("origin/main", RemoteBranch),
            ],
        ),
        entry("b", &["a"], &[("feature", LocalBranch)]),
        entry("c", &["a"], &[("v1", Tag)]),
        entry("a", &[], &[]),
    ]);
    assert_eq!(log.lanes, 2);
    let painted = frame(
        &egui::Context::default(),
        &log,
        egui::vec2(700.0, 400.0),
        vec![],
    );
    // One curve leaves the merge for lane 1; one joins it back at the fork.
    let curves = painted
        .shapes
        .iter()
        .filter(|shape| matches!(shape, egui::Shape::CubicBezier(_)))
        .count();
    assert_eq!(curves, 2);
    let colors: Vec<egui::Color32> = dots(&painted.shapes).iter().map(|d| d.fill).collect();
    assert_eq!(
        colors,
        [lane_color(0), lane_color(1), lane_color(0), lane_color(0)]
    );
    let texts = texts(&painted.shapes);
    for want in ["HEAD", "main", "origin/main", "feature", "v1", "subject m"] {
        assert!(texts.iter().any(|text| text == want), "{want}: {texts:?}");
    }
}

#[test]
fn clicking_a_row_returns_its_commit() {
    let log = log(vec![
        entry("c", &["b"], &[]),
        entry("b", &["a"], &[]),
        entry("a", &[], &[]),
    ]);
    let ctx = egui::Context::default();
    let size = egui::vec2(400.0, 300.0);
    let first = frame(&ctx, &log, size, vec![]);
    let pos = dots(&first.shapes)[1].center;
    let press = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    };
    frame(&ctx, &log, size, vec![egui::Event::PointerMoved(pos)]);
    frame(&ctx, &log, size, vec![press(true)]);
    let released = frame(&ctx, &log, size, vec![press(false)]);
    assert_eq!(released.clicked.as_deref(), Some("b"));
}

#[test]
fn ref_pills_that_do_not_fit_fold_into_a_count() {
    let names: Vec<String> = (0..12).map(|index| format!("release-{index}")).collect();
    let refs: Vec<(&str, RefLabelKind)> = names
        .iter()
        .map(|name| (name.as_str(), RefLabelKind::Tag))
        .collect();
    let log = log(vec![entry("a", &[], &refs)]);
    let painted = frame(
        &egui::Context::default(),
        &log,
        egui::vec2(320.0, 200.0),
        vec![],
    );
    let texts = texts(&painted.shapes);
    // A pill cut with an ellipsis still counts as shown.
    let shown = texts.iter().filter(|t| t.starts_with("rel")).count();
    assert!(shown >= 1, "{texts:?}");
    let more = format!("+{}", names.len().saturating_sub(shown));
    assert!(texts.contains(&more), "{more}: {texts:?}");
}

#[test]
fn wide_graphs_narrow_their_lanes_then_clip() {
    let geometry = |lanes| LaneGeometry::new(lanes, 400.0);
    let row = |width: usize| GraphRow {
        col: u16::try_from(width.saturating_sub(1)).unwrap(),
        ..GraphRow::default()
    };
    // The graph may take 40% of 400 px: 160 px.
    let fits = geometry(3);
    assert_eq!(fits.pitch, 12.0);
    // Each row is as wide as its own lanes.
    assert_eq!(fits.row_width(&row(1)), 12.0);
    assert_eq!(fits.row_width(&row(3)), 36.0);
    // 20 lanes need 240 px, so every lane narrows to 8 px.
    let narrow = geometry(20);
    assert_eq!(narrow.pitch, 8.0);
    assert_eq!(narrow.row_width(&row(20)), 160.0);
    // Past the narrowest pitch the widest rows clip at 160 px.
    let clipped = geometry(100);
    assert_eq!(clipped.pitch, 5.0);
    assert_eq!(clipped.row_width(&row(2)), 10.0);
    assert_eq!(clipped.row_width(&row(100)), 160.0);
    // An empty log still keeps one full lane.
    assert_eq!(geometry(0).pitch, 12.0);
}

#[test]
fn subjects_start_after_each_rows_own_lanes() {
    // Lane 1 joins lane 0 at `a`; below that, `r` uses one lane again.
    let log = log(vec![
        entry("m", &["a", "b"], &[]),
        entry("b", &["a"], &[]),
        entry("a", &["r"], &[]),
        entry("r", &[], &[]),
    ]);
    let painted = frame(
        &egui::Context::default(),
        &log,
        egui::vec2(500.0, 300.0),
        vec![],
    );
    let left = |subject: &str| {
        painted
            .shapes
            .iter()
            .find_map(|shape| match shape {
                egui::Shape::Text(text) if text.galley.text() == subject => Some(text.pos.x),
                _ => None,
            })
            .unwrap()
    };
    assert!(left("subject b") > left("subject r"));
    assert_eq!(left("subject m"), left("subject b"));
    assert_eq!(left("subject a"), left("subject b"));
}
