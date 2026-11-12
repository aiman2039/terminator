use super::page::{
    Scenes, href, own_url, read_library, read_scene, write_image, write_library, write_scene,
};
use std::path::Path;

const SCENE: &[u8] = br#"{"type":"excalidraw","version":2,"elements":[]}"#;

#[cfg(feature = "webview")]
#[test]
fn excalidraw_files_open_as_drawings() {
    use super::supported;
    assert!(supported(Path::new("docs/flow.excalidraw")));
    assert!(supported(Path::new("FLOW.EXCALIDRAW")));
    assert!(!supported(Path::new("flow.excalidraw.png")));
    assert!(!supported(Path::new("page.html")));
}

#[test]
fn drawing_view_stays_on_the_bundled_page() {
    let page = href("tab 1", true);
    assert_eq!(
        page,
        "terminator-draw://localhost/index.html?id=tab+1&theme=dark"
    );
    assert!(own_url(&page));
    assert!(own_url("terminator-draw://localhost/app.js"));
    for blocked in [
        "https://example.com/",
        "file:///etc/passwd",
        "terminator-draw://localhost.evil/",
        "terminator-draw://other/index.html",
    ] {
        assert!(!own_url(blocked), "{blocked}");
    }
}

#[test]
fn catalog_add_link_reaches_only_the_named_drawing() {
    use super::page::{catalog_url, install_request};
    assert!(catalog_url(
        "https://libraries.excalidraw.com/?target=_blank"
    ));
    assert!(!catalog_url("https://libraries.excalidraw.com.evil/"));
    let link = "terminator-draw://localhost/index.html?id=tab-1#addLibrary=https%3A%2F%2Flibraries.excalidraw.com%2Flibraries%2Fa%2Fb.excalidrawlib&token=t1";
    let (key, script) = install_request(link).unwrap();
    assert_eq!(key, "tab-1");
    assert_eq!(
        script,
        r#"location.hash = "addLibrary=https%3A%2F%2Flibraries.excalidraw.com%2Flibraries%2Fa%2Fb.excalidrawlib&token=t1";"#
    );
    for refused in [
        "terminator-draw://localhost/index.html?id=tab-1#addLibrary=https%3A%2F%2Fevil.example%2Fa.excalidrawlib",
        "terminator-draw://localhost/index.html?id=tab-1",
        "terminator-draw://localhost/index.html#addLibrary=https%3A%2F%2Flibraries.excalidraw.com%2Fa",
        "https://example.com/index.html?id=tab-1#addLibrary=https%3A%2F%2Flibraries.excalidraw.com%2Fa",
    ] {
        assert!(install_request(refused).is_none(), "{refused}");
    }
    // A quote in the link cannot leave the script string.
    let quoted = "terminator-draw://localhost/index.html?id=a#addLibrary=https%3A%2F%2Flibraries.excalidraw.com%2Fa&x=\";alert(1)//";
    if let Some((_, script)) = install_request(quoted) {
        assert!(!script.contains("\";alert"), "{script}");
    }
}

#[test]
fn missing_scene_file_is_an_empty_drawing() {
    let dir = tempfile::tempdir().unwrap();
    let reply = read_scene(&dir.path().join("new.excalidraw"), "").unwrap();
    let reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(reply["stamp"], "none");
    assert_eq!(reply["text"], "");
}

#[test]
fn save_replaces_the_file_and_an_unchanged_file_sends_no_text() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.excalidraw");
    let stamp = write_scene(&path, SCENE).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), SCENE);
    let same: serde_json::Value =
        serde_json::from_str(&read_scene(&path, &stamp).unwrap()).unwrap();
    assert_eq!(same["stamp"], stamp.as_str());
    assert!(same.get("text").is_none());
    // Another program writes the file: the next poll carries the text.
    std::fs::write(&path, br#"{"type":"excalidraw","elements":[{"id":"x"}]}"#).unwrap();
    let changed: serde_json::Value =
        serde_json::from_str(&read_scene(&path, &stamp).unwrap()).unwrap();
    assert!(changed["text"].as_str().unwrap().contains(r#""id":"x""#));
}

#[test]
fn save_refuses_text_that_is_not_a_scene() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.excalidraw");
    std::fs::write(&path, SCENE).unwrap();
    assert!(write_scene(&path, b"not json").is_err());
    assert!(write_scene(&path, br#"{"type":"other"}"#).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), SCENE);
}

#[test]
fn image_export_writes_beside_the_scene_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.excalidraw");
    assert!(write_image(&path, b"<svg/>", "png").is_err());
    assert!(write_image(&path, b"\x89PNG\r\n\x1a\nrest", "svg").is_err());
    let name = write_image(&path, b"\x89PNG\r\n\x1a\nrest", "png").unwrap();
    assert_eq!(name, "a.excalidraw.png");
    assert_eq!(
        write_image(&path, b"<svg></svg>", "svg").unwrap(),
        "a.excalidraw.svg"
    );
    assert!(dir.path().join("a.excalidraw.png").is_file());
    assert!(dir.path().join("a.excalidraw.svg").is_file());
    assert!(!path.exists());
}

#[test]
fn library_is_empty_until_saved_and_refuses_other_text() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.excalidrawlib");
    assert_eq!(read_library(&path).unwrap(), "");
    assert!(write_library(&path, br#"{"type":"excalidraw"}"#).is_err());
    assert!(!path.exists());
    let library = br#"{"type":"excalidrawlib","version":2,"libraryItems":[]}"#;
    write_library(&path, library).unwrap();
    assert_eq!(read_library(&path).unwrap().as_bytes(), library);
}

#[test]
fn handler_serves_only_open_drawings() {
    let scenes = Scenes::default();
    scenes.insert("a", Path::new("/tmp/a.excalidraw"));
    scenes.insert("b", Path::new("/tmp/b.excalidraw"));
    scenes.retain(|key| key == "a");
    assert_eq!(
        scenes.path("a").as_deref(),
        Some(Path::new("/tmp/a.excalidraw"))
    );
    assert!(scenes.path("b").is_none());
}
