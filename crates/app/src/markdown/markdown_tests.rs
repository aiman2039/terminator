use super::html::{Link, prepare};
use super::previews::{Preview, Previews};
#[cfg(unix)]
use super::source::{MAX_DOCUMENT, Source, read_source, supported};
use super::source::{Mode, Revision, Snapshot};
use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use pulldown_cmark::{Event, Parser, Tag};
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
            path: "/docs/read me.md".into(),
            text: text.into(),
            revision: None,
            paused: false,
        }
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
    fn diff_snapshots_resolve_links_refresh_and_release_resources() {
        let ctx = egui::Context::default();
        let mut previews = Previews::new(&ctx);
        let path = Path::new("/repo/docs/readme.md");
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
            Some(Link::File("/repo/next.md".into()))
        );
        assert_eq!(doc.links["command:run"], None);
        assert!(
            doc.images
                .contains("markdown-image:file:///repo/docs/images/a.png")
        );
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
            Some(Link::File("/docs/nested/next file.md".into()))
        );
        assert!(
            doc.images
                .contains("markdown-image:file:///docs/images/a%20)%20b.png"),
            "{:?}",
            doc.images
        );
        assert!(doc.images.contains("markdown-image:file:///photo.png"));
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
}
