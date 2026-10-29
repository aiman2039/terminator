use super::html::{Link, prepare};
use super::previews::{Preview, Previews};
#[cfg(unix)]
use super::source::{MAX_DOCUMENT, Source, read_source, supported};
use super::source::{Mode, Revision, Snapshot};
use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use pulldown_cmark::{Event, Parser, Tag};
#[cfg(unix)]
use std::path::Path;
#[cfg(unix)]
use std::time::Duration;
#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use std::io::Read;

    fn snapshot(text: &str) -> Snapshot {
        Snapshot {
            path: crate::test_path("/docs/read me.md"),
            text: text.into(),
            revision: None,
            paused: false,
        }
    }

    #[test]
    fn preview_places_source_newlines_and_html_breaks_on_separate_rows() {
        fn rows(shape: &egui::Shape, out: &mut Vec<(f32, String)>) {
            match shape {
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        rows(shape, out);
                    }
                }
                egui::Shape::Text(text) => {
                    for row in &text.galley.rows {
                        out.push((
                            text.pos.y + row.pos.y,
                            row.glyphs.iter().map(|glyph| glyph.chr).collect(),
                        ));
                    }
                }
                _ => {}
            }
        }

        for (source, separate_lines) in [
            "Alpha\nBeta\nGamma",
            "Alpha\r\nBeta\r\nGamma",
            "Alpha<br>Beta<BR />Gamma",
            "Alpha<br><br>Beta<br>Gamma",
            "<p>Alpha<br/>Beta<br>Gamma</p>",
            "| Header |\n| --- |\n| Alpha<br>Beta<br/>Gamma |",
            "| Alpha<br>Beta<br>Gamma | Other |\n| --- | --- |\n| Body | Cell |",
            "| Header | Other |\n| --- | --- |\n| Alpha<br>Beta<br>Gamma | Cell |",
            "> Alpha\n> Beta\n> Gamma",
            "- Alpha\n  Beta\n  Gamma",
            "Alpha  \nBeta\\\nGamma",
        ]
        .into_iter()
        .map(|source| (source, true))
        .chain([(
            "| Header | Other |\n| --- | --- |\n| Alpha **Beta** Gamma | Cell |",
            false,
        )]) {
            let mut preview = Preview::default();
            preview.apply(Ok(prepare(snapshot(source))));
            let ctx = egui::Context::default();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 700.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    preview.show(ui, "line-breaks");
                },
            );
            output.textures_delta.clear();
            let mut painted = Vec::new();
            for clipped in &output.shapes {
                rows(&clipped.shape, &mut painted);
            }
            let positions: Vec<_> = ["Alpha", "Beta", "Gamma"]
                .iter()
                .map(|word| {
                    painted
                        .iter()
                        .find(|(_, text)| text.contains(word))
                        .unwrap_or_else(|| panic!("Missing {word}: {source:?}: {painted:?}"))
                        .0
                })
                .collect();
            assert!(
                positions.windows(2).all(|pair| if separate_lines {
                    pair[0] < pair[1]
                } else {
                    pair[0] == pair[1]
                }),
                "Line breaks lost: {source:?}: {painted:?}"
            );
        }
    }

    #[test]
    fn ordered_list_numbers_align_with_the_first_text_line() {
        fn text_rows(shape: &egui::Shape, out: &mut Vec<(f32, String)>) {
            match shape {
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        text_rows(shape, out);
                    }
                }
                egui::Shape::Text(text) => {
                    for row in &text.galley.rows {
                        out.push((
                            text.pos.y
                                + row.pos.y
                                + row.glyphs.first().map_or(0.0, |glyph| glyph.pos.y),
                            row.glyphs.iter().map(|glyph| glyph.chr).collect(),
                        ));
                    }
                }
                _ => {}
            }
        }

        for source in [
            "1. Alpha\n2. Beta",
            "1. **Alpha**\n2. Beta",
            "1. `Alpha`\n2. `Beta`",
            "1. Alpha<br>continuation\n2. Beta",
            "1. Alpha\n   continuation\n2. Beta",
            "1. Alpha with enough words to wrap onto several lines in a narrow preview\n2. Beta",
            "1. Alpha\n\n2. Beta",
            "9. Alpha\n10. Beta",
            "1. Parent\n   1. Alpha\n   2. Beta",
        ] {
            let ctx = egui::Context::default();
            crate::appearance::install(&ctx);
            let mut preview = Preview::default();
            preview.apply(Ok(prepare(snapshot(source))));
            for _ in 0..3 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(340.0, 900.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        // A large control height must not change the text baseline.
                        ui.spacing_mut().interact_size.y = 32.0;
                        preview.show(ui, "ordered-list-alignment");
                    },
                );
                output.textures_delta.clear();
                let mut painted = Vec::new();
                for clipped in &output.shapes {
                    text_rows(&clipped.shape, &mut painted);
                }
                let numbered_rows: Vec<_> = painted
                    .iter()
                    .filter(|(_, text)| {
                        text.strip_suffix('.')
                            .is_some_and(|number| number.parse::<u64>().is_ok())
                    })
                    .collect();
                for word in ["Alpha", "Beta"] {
                    let (text_y, _) = painted
                        .iter()
                        .find(|(_, text)| text.contains(word))
                        .unwrap_or_else(|| panic!("Missing {word}: {source:?}: {painted:?}"));
                    assert!(
                        numbered_rows
                            .iter()
                            .any(|(number_y, _)| (text_y - number_y).abs() <= 1.0),
                        "Number and text do not align: {source:?}: {painted:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn extra_empty_lines_add_visible_space_between_blocks() {
        fn positions(shape: &egui::Shape, out: &mut Vec<(f32, String)>) {
            match shape {
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        positions(shape, out);
                    }
                }
                egui::Shape::Text(text) => {
                    for row in &text.galley.rows {
                        out.push((
                            text.pos.y + row.pos.y,
                            row.glyphs.iter().map(|glyph| glyph.chr).collect(),
                        ));
                    }
                }
                _ => {}
            }
        }

        for (before, after, separator) in [
            ("Alpha", "Beta", "\n"),
            ("Alpha", "Beta", "\r\n"),
            ("# Alpha", "Beta", "\n"),
            ("- Alpha", "- Beta", "\n"),
            ("> Alpha", " Beta", "\n>"),
            ("Alpha", "Beta", "<br>"),
            ("```text\nAlpha\n```", "Beta", "\n"),
        ] {
            let mut gaps = Vec::new();
            for newlines in 2..=4 {
                let source = format!("{before}{}{after}", separator.repeat(newlines));
                let ctx = egui::Context::default();
                crate::appearance::install(&ctx);
                let mut preview = Preview::default();
                preview.apply(Ok(prepare(snapshot(&source))));
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(800.0, 900.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        preview.show(ui, "blank-line-spacing");
                    },
                );
                output.textures_delta.clear();
                let mut painted = Vec::new();
                for clipped in &output.shapes {
                    positions(&clipped.shape, &mut painted);
                }
                let y = |word: &str| {
                    painted
                        .iter()
                        .find(|(_, text)| text.contains(word))
                        .unwrap_or_else(|| panic!("Missing {word}: {source:?}: {painted:?}"))
                        .0
                };
                gaps.push(y("Beta") - y("Alpha"));
            }
            assert!(
                gaps.windows(2).all(|pair| pair[1] - pair[0] >= 16.0),
                "Extra empty lines collapsed: {before:?}, {after:?}, {separator:?}: {gaps:?}"
            );
        }
    }

    #[test]
    fn line_break_conversion_preserves_code_and_table_structure() {
        let code =
            "`Alpha<br>Beta`\n\n```text\nAlpha\n<br>\nBeta\n```\n\n    Alpha\n    <br>\n    Beta\n";
        assert_eq!(prepare(snapshot(code)).text, code);

        let doc = prepare(snapshot(
            "| A | B |\n| --- | --- |\n| One<br><br>Two | Three |",
        ));
        let events: Vec<_> = Parser::new_ext(&doc.text, pulldown_cmark::Options::all()).collect();
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, Event::Start(Tag::TableCell)))
                .count(),
            4
        );
        let text: String = events
            .iter()
            .filter_map(|event| match event {
                Event::Text(text) => Some(text.as_ref()),
                _ => None,
            })
            .collect();
        assert!(text.contains("One\n\nTwo"), "{text:?}");
    }

    #[test]
    fn readme_style_html_renders_as_text_not_literal_tags() {
        let doc = prepare(snapshot(
            "<p align=\"center\">\n  <img src=\"logo.png\" alt=\"Terminator app icon\" width=\"128\" height=\"128\">\n</p>\n\n<h1 align=\"center\">Terminator</h1>\n\n<p align=\"center\">Your terminals.<br>For macOS and Linux.</p>\n\n<a href=\"https://example.com/ci\"><img src=\"https://example.com/badge.svg\" alt=\"CI\"></a>",
        ));
        assert!(
            !doc.text.contains('<'),
            "raw HTML leaked into preview: {:?}",
            doc.text
        );
        assert!(doc.text.contains("# Terminator"), "{:?}", doc.text);
        assert!(
            doc.text.contains("Image: Terminator app icon"),
            "{:?}",
            doc.text
        );
        assert!(doc.text.contains("Image: CI"), "{:?}", doc.text);
        // The converted document must still render without panicking.
        let ctx = egui::Context::default();
        let mut cache = CommonMarkCache::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600.0, 800.0),
                )),
                ..Default::default()
            },
            |ui| {
                CommonMarkViewer::new().show(ui, &mut cache, &doc.text);
            },
        );
        output.textures_delta.clear();
    }

    #[test]
    fn mermaid_preview_uploads_diagram_and_keeps_links() {
        let doc = prepare(snapshot(
            "# Flow\n\n```mermaid\nflowchart LR\n  A --> B\n```\n\n[Next](next.md)\n",
        ));
        assert_eq!(doc.mermaid.len(), 1);
        let mut preview = Preview::default();
        preview.apply(Ok(doc));
        let ctx = egui::Context::default();
        crate::appearance::install(&ctx);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 700.0),
                )),
                ..Default::default()
            },
            |ui| {
                preview.show(ui, "mermaid-preview");
            },
        );
        assert_eq!(preview.mermaid_textures.len(), 1);
        assert_eq!(preview.cache.get_link_hook("next.md"), Some(false));
        output.textures_delta.clear();
    }

    #[test]
    fn diff_snapshots_resolve_links_refresh_and_release_resources() {
        let ctx = egui::Context::default();
        let mut previews = Previews::new(&ctx);
        let path = &crate::test_path("/repo/docs/readme.md");
        previews.begin_frame();
        previews.snapshot(
            "diff:left",
            path,
            "[Next](../next.md) ![pic](images/a.png) [bad](command:run)",
        );
        previews.wait_prepared();
        let preview = &previews.entries["diff:left"];
        let doc = preview.document.as_ref().unwrap();
        assert_eq!(
            doc.links["../next.md"],
            Some(Link::File(crate::test_path("/repo/next.md")))
        );
        assert_eq!(doc.links["command:run"], None);
        assert!(doc.images.contains(&format!(
            "markdown-image:{}",
            url::Url::from_file_path(crate::test_path("/repo/docs/images/a.png")).unwrap()
        )));
        assert_eq!(preview.cache.get_link_hook("../next.md"), Some(false));
        previews.snapshot("diff:right", path, "# Right snapshot");
        previews.end_frame(&ctx);
        assert_eq!(previews.entries.len(), 2);
        previews.begin_frame();
        previews.snapshot("diff:left", path, "[New](new.md)");
        previews.wait_prepared();
        let preview = &previews.entries["diff:left"];
        assert!(
            !preview
                .document
                .as_ref()
                .unwrap()
                .links
                .contains_key("../next.md")
        );
        assert_eq!(preview.cache.get_link_hook("../next.md"), None);
        assert_eq!(preview.cache.get_link_hook("new.md"), Some(false));
        previews.end_frame(&ctx);
        assert_eq!(previews.entries.len(), 1);
        previews.begin_frame();
        previews.end_frame(&ctx);
        assert!(previews.entries.is_empty());
    }

    #[cfg(unix)]
    #[test]
    #[cfg(unix)]
    fn blocking_editor_uses_saved_file_and_preserves_the_last_unsaved_preview() {
        use std::{io::Write, os::unix::net::UnixListener};
        let dir = tempfile::tempdir().unwrap();
        let source = Source {
            session: "editor".into(),
            path: dir.path().join("README.md"),
            socket: dir.path().join("nvim.sock"),
        };
        std::fs::write(&source.path, "# Saved file").unwrap();
        let listener = UnixListener::bind(&source.socket).unwrap();
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut peer, _) = listener.accept().unwrap();
                let request: serde_json::Value = rmp_serde::from_read(&mut peer).unwrap();
                assert_eq!(request[2], "nvim_get_mode");
                peer.write_all(
                    &rmp_serde::to_vec(&(
                        1,
                        1,
                        serde_json::Value::Null,
                        serde_json::json!({"mode":"rm","blocking":true}),
                    ))
                    .unwrap(),
                )
                .unwrap();
                // The client must close without queuing an evaluation behind the prompt.
                let mut byte = [0];
                assert_eq!(peer.read(&mut byte).unwrap(), 0);
            }
        });
        let saved = read_source(&source, None).unwrap().unwrap();
        assert_eq!(saved.text, "# Saved file");
        assert_eq!(prepare(saved).status, "Saved file · live preview paused");
        let mut dirty = snapshot("# Unsaved edits");
        dirty.revision = Some(Revision {
            buffer: 1,
            tick: 2,
            modified: true,
        });
        let preserved = read_source(&source, Some(&dirty)).unwrap().unwrap();
        assert_eq!(preserved.text, "# Unsaved edits");
        assert_eq!(
            prepare(preserved).status,
            "Unsaved preview · live updates paused"
        );
        assert_eq!(
            std::fs::read_to_string(&source.path).unwrap(),
            "# Saved file"
        );
        server.join().unwrap();
    }

    #[test]
    fn markdown_defaults_to_preview_and_explicit_edit_mode_is_persisted() {
        let mut preferences = crate::preferences::UiPreferences::default();
        assert_eq!(
            preferences
                .markdown_modes
                .get("new")
                .copied()
                .unwrap_or_default(),
            Mode::Preview
        );
        preferences
            .markdown_modes
            .insert("editor".into(), Mode::Edit);
        let restored: crate::preferences::UiPreferences =
            serde_json::from_str(&serde_json::to_string(&preferences).unwrap()).unwrap();
        assert_eq!(restored.markdown_modes["editor"], Mode::Edit);
    }

    #[test]
    #[cfg(unix)]
    fn refresh_during_a_prompt_keeps_the_cached_unsaved_document() {
        use std::{io::Write, os::unix::net::UnixListener, time::Instant};
        let dir = tempfile::tempdir().unwrap();
        let source = Source {
            session: "editor".into(),
            path: dir.path().join("README.md"),
            socket: dir.path().join("nvim.sock"),
        };
        std::fs::write(&source.path, "# Saved").unwrap();
        let listener = UnixListener::bind(&source.socket).unwrap();
        let path = source.path.clone();
        let server = std::thread::spawn(move || {
            for blocked in [false, true] {
                let (mut peer, _) = listener.accept().unwrap();
                peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let request: serde_json::Value = rmp_serde::from_read(&mut peer).unwrap();
                assert_eq!(request[2], "nvim_get_mode");
                peer.write_all(
                    &rmp_serde::to_vec(&(
                        1,
                        1,
                        serde_json::Value::Null,
                        serde_json::json!({"mode":"n","blocking":blocked}),
                    ))
                    .unwrap(),
                )
                .unwrap();
                if !blocked {
                    let request: serde_json::Value = rmp_serde::from_read(&mut peer).unwrap();
                    assert_eq!(request[2], "nvim_exec_lua");
                    let response = serde_json::json!({"path":path,"text":"# Unsaved","revision":{"buffer":1,"tick":2,"modified":true}}).to_string();
                    peer.write_all(
                        &rmp_serde::to_vec(&(1, 2, serde_json::Value::Null, response)).unwrap(),
                    )
                    .unwrap();
                }
            }
        });
        let ctx = egui::Context::default();
        let mut previews = Previews::new(&ctx);
        for paused in [false, true] {
            if paused {
                previews.refresh(&ctx);
            }
            let started = Instant::now();
            loop {
                previews.begin_frame();
                previews.retain("editor");
                previews.watch(source.clone());
                previews.end_frame(&ctx);
                if previews.entries["editor"]
                    .document
                    .as_ref()
                    .is_some_and(|d| d.status.contains("paused") == paused)
                {
                    break;
                }
                assert!(
                    started.elapsed() < Duration::from_secs(2),
                    "Preview error: {:?}",
                    previews.entries["editor"].error
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(
                previews.entries["editor"].document.as_ref().unwrap().text,
                "# Unsaved"
            );
        }
        server.join().unwrap();
    }

    #[test]
    fn relative_links_and_images_resolve_against_the_document() {
        let doc = prepare(snapshot(
            "[Next](nested/next%20file.md)\n\n![a **picture**](<images/a ) b.png>)\n\n![Reference][photo]\n\n[photo]: ../photo.png\n\n`![literal](no.png)`",
        ));
        assert_eq!(
            doc.links["nested/next%20file.md"],
            Some(Link::File(crate::test_path("/docs/nested/next file.md")))
        );
        assert!(
            doc.images.contains(&format!(
                "markdown-image:{}",
                url::Url::from_file_path(crate::test_path("/docs/images/a ) b.png")).unwrap()
            )),
            "{:?}",
            doc.images
        );
        assert!(doc.images.contains(&format!(
            "markdown-image:{}",
            url::Url::from_file_path(crate::test_path("/photo.png")).unwrap()
        )));
        let rendered_images = Parser::new(&doc.text)
            .filter(|e| matches!(e, Event::Start(Tag::Image { .. })))
            .count();
        assert_eq!(rendered_images, 2);
        assert!(doc.text.contains("`![literal](no.png)`"));
        assert!(doc.text.contains("![a picture](<markdown-image:"));
    }

    #[test]
    fn preview_links_do_not_execute_arbitrary_url_schemes_or_fetch_remote_images() {
        let doc = prepare(snapshot(
            "[bad](javascript:alert) [shell](command:run) [web](https://example.com) [section](#title) ![remote](https://example.com/a.png)",
        ));
        assert_eq!(doc.links["javascript:alert"], None);
        assert_eq!(doc.links["command:run"], None);
        assert_eq!(
            doc.links["https://example.com"],
            Some(Link::Web("https://example.com/".into()))
        );
        assert!(!doc.links.contains_key("#title"));
        assert!(doc.images.is_empty());
        assert!(doc.text.contains("Image: remote"));
    }

    #[test]
    fn bare_web_urls_become_links_outside_code_and_existing_links() {
        let doc = prepare(snapshot(
            "See https://example.com/a_b__c+d now.\n\n[x](https://example.com/y) `https://example.com/code`\n\n```\nhttps://example.com/block\n```\n\nftp://example.com/file",
        ));
        assert!(doc.text.contains("See <https://example.com/a_b__c+d> now."));
        assert_eq!(
            doc.links["https://example.com/a_b__c+d"],
            Some(Link::Web("https://example.com/a_b__c+d".into()))
        );
        assert!(doc.text.contains("[x](https://example.com/y)"));
        assert!(doc.text.contains("`https://example.com/code`"));
        assert!(doc.text.contains("\nhttps://example.com/block\n"));
        assert!(!doc.text.contains("<ftp:"));
    }

    #[cfg(unix)]
    #[test]
    #[cfg(unix)]
    fn saved_file_preview_reports_changes_missing_files_and_size_limits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.MD");
        let source = Source {
            session: "fixture".into(),
            path: path.clone(),
            socket: dir.path().join("missing.sock"),
        };
        std::fs::write(&path, "# First").unwrap();
        let first = read_source(&source, None).unwrap().unwrap();
        assert_eq!(first.text, "# First");
        assert_eq!(prepare(first).status, "Saved file");
        std::fs::write(&path, "# Changed").unwrap();
        assert_eq!(
            read_source(&source, None).unwrap().unwrap().text,
            "# Changed"
        );
        std::fs::File::create(&path)
            .unwrap()
            .set_len(MAX_DOCUMENT as u64 + 1)
            .unwrap();
        assert!(
            read_source(&source, None)
                .unwrap_err()
                .to_string()
                .contains("1 MiB")
        );
        std::fs::write(&path, [255]).unwrap();
        assert!(
            read_source(&source, None)
                .unwrap_err()
                .to_string()
                .contains("UTF-8")
        );
        std::fs::remove_file(&path).unwrap();
        assert!(read_source(&source, None).is_err());
        assert!(supported(&path));
        assert!(!supported(Path::new("file.mdx")));
        assert!(!supported(Path::new("file.rs")));
    }

    #[test]
    fn stale_preview_updates_cannot_replace_a_newer_watch() {
        let ctx = egui::Context::default();
        let mut previews = Previews::new(&ctx);
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        previews.results = rx;
        previews.generation = 2;
        previews.entries.insert("editor".into(), Preview::default());
        tx.try_send((1, "editor".into(), Ok(prepare(snapshot("old")))))
            .unwrap();
        previews.begin_frame();
        assert!(previews.entries["editor"].document.is_none());
        tx.try_send((2, "editor".into(), Ok(prepare(snapshot("new")))))
            .unwrap();
        previews.begin_frame();
        assert_eq!(
            previews.entries["editor"].document.as_ref().unwrap().text,
            "new"
        );
        previews.end_frame(&ctx);
        assert!(previews.entries.is_empty());
    }

    #[test]
    fn refresh_failure_keeps_last_preview_and_recovery_clears_the_error() {
        let mut preview = Preview::default();
        let mut dirty = snapshot("# Unsaved");
        dirty.revision = Some(Revision {
            buffer: 1,
            tick: 3,
            modified: true,
        });
        preview.apply(Ok(prepare(dirty)));
        assert_eq!(preview.document.as_ref().unwrap().status, "Unsaved changes");
        preview.apply(Err("Editor temporarily unavailable".into()));
        assert_eq!(preview.document.as_ref().unwrap().text, "# Unsaved");
        preview.apply(Ok(prepare(snapshot("# Recovered"))));
        assert!(preview.error.is_none());
    }

    #[test]
    fn clicking_preview_suspends_editor_keyboard_focus() {
        let ctx = egui::Context::default();
        let mut preview = Preview {
            preview_rect: egui::Rect::from_min_max(
                egui::pos2(100.0, 0.0),
                egui::pos2(200.0, 100.0),
            ),
            ..Default::default()
        };
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::PointerButton {
                    pos: egui::pos2(150.0, 40.0),
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                }],
                ..Default::default()
            },
            |ui| preview.pointer_focus(ui),
        );
        output.textures_delta.clear();
        assert!(!preview.editor_focused);
    }
    #[test]
    fn remote_snapshots_never_load_local_file_links_or_images() {
        let ctx = egui::Context::default();
        let mut previews = Previews::new(&ctx);
        previews.begin_frame();
        previews.snapshot("remote-md",&crate::test_path("/__terminator_remote__/readme.md"),"![local](file:///private/secret.png) [local](file:///private/secret.txt) [web](https://example.com)");
        previews.wait_prepared();
        let doc = previews.entries["remote-md"].document.as_ref().unwrap();
        assert!(doc.images.is_empty());
        assert_eq!(doc.links["file:///private/secret.txt"], None);
        assert_eq!(
            doc.links["https://example.com"],
            Some(Link::Web("https://example.com/".into()))
        );
    }
}
