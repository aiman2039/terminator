use super::*;

fn target(text: &str) -> LinkTarget {
    LinkTarget {
        text: text.into(),
        rects: vec![egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(8.0, 16.0),
        )],
        hyperlink: false,
    }
}

fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

#[test]
fn mode_input_leaves_no_keyboard_event_for_the_terminal() {
    let mut events = vec![
        key(egui::Key::J, egui::Modifiers::NONE),
        egui::Event::Text("j".into()),
        key(egui::Key::Escape, egui::Modifiers::NONE),
        key(egui::Key::D, egui::Modifiers::CTRL),
        egui::Event::Text("\u{4}".into()),
        egui::Event::Paste("rm -rf /".into()),
        egui::Event::Copy,
        egui::Event::PointerMoved(egui::pos2(1.0, 1.0)),
    ];
    let keys = take_mode_keys(&mut events);
    assert_eq!(
        keys,
        [
            ModeKey::Char('j'),
            ModeKey::Named(egui::Key::Escape),
            ModeKey::Ctrl('d'),
        ]
    );
    assert_eq!(events, [egui::Event::PointerMoved(egui::pos2(1.0, 1.0))]);
}

#[test]
fn labels_are_unique_and_none_is_a_prefix_of_another() {
    assert_eq!(labels(3), ["a", "s", "d"]);
    let many = labels(40);
    assert_eq!(many.len(), 40);
    assert!(many.iter().all(|label| label.len() == 2));
    let unique: std::collections::HashSet<_> = many.iter().collect();
    assert_eq!(unique.len(), 40);
}

#[test]
fn hints_cover_links_paths_file_lines_and_hashes_only() {
    for text in [
        "https://example.com/a",
        "src/main.rs",
        "~/notes",
        "main.rs:12:4",
        "Cargo.toml",
        "9fceb02d0ae598e95dc970b74767f19372d61af8",
        "359487c",
    ] {
        assert!(hintable(&target(text)), "{text}");
    }
    for text in ["hello", "1.5", "x", "deadbeefcafe", "ABCDEF1"] {
        assert!(!hintable(&target(text)), "{text}");
    }
}

#[test]
fn typing_a_label_picks_its_item_and_shift_copies() {
    let mut hints = Hints::new(vec![target("a.rs"), target("b.rs"), target("plain")]);
    assert_eq!(hints.items.len(), 2, "plain words get no label");
    assert_eq!(hints.key(ModeKey::Char('x')), HintStep::Wait);
    assert!(
        hints.typed.is_empty(),
        "a letter no label starts with is dropped"
    );
    assert_eq!(
        hints.key(ModeKey::Char('s')),
        HintStep::Pick {
            index: 1,
            copy: false
        }
    );
    hints.typed.clear();
    assert_eq!(
        hints.key(ModeKey::Char('A')),
        HintStep::Pick {
            index: 0,
            copy: true
        }
    );
    assert_eq!(
        hints.key(ModeKey::Named(egui::Key::Escape)),
        HintStep::Leave
    );
}
