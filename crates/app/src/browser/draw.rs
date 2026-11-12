//! `.excalidraw` drawings: a Browser tab that shows the bundled Excalidraw
//! page from the `terminator-draw` scheme. The page reads and writes only
//! the scene file of its own tab; it has no other way into the app.
use crate::browser::BrowserTarget;
use std::path::Path;

#[cfg(feature = "webview")]
pub(crate) use page::{SCHEME, Scenes, catalog_url, install_request, own_url, pane_href};

/// False without the `webview` feature: the file then opens as text.
pub(crate) fn supported(path: &Path) -> bool {
    cfg!(feature = "webview")
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("excalidraw"))
}

pub(crate) fn is_drawing(target: &BrowserTarget) -> bool {
    target.file().is_some_and(supported)
}

/// The page side: only builds with a webview (and tests) use it.
#[cfg(any(feature = "webview", test))]
mod page {
    use std::{
        collections::HashMap,
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
    };

    #[cfg(feature = "webview")]
    pub(crate) const SCHEME: &str = "terminator-draw";
    pub(super) const ORIGIN: &str = "terminator-draw://localhost";
    /// In the data folder; the bundled shapes apply until it exists.
    const LIBRARY_FILE: &str = "excalidraw-library.excalidrawlib";

    /// Page address of the drawing in tab `key`.
    pub(crate) fn href(key: &str, dark: bool) -> String {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("id", key)
            .append_pair("theme", if dark { "dark" } else { "light" })
            .finish();
        format!("{ORIGIN}/index.html?{query}")
    }

    /// Address a pane loads: the drawing page for a drawing (its scene file
    /// becomes readable by that page), else the target's own address.
    pub(crate) fn pane_href(
        scenes: &Scenes,
        (key, target): (&str, &super::BrowserTarget),
        (dark, data_dir): (bool, &Path),
    ) -> Result<String, String> {
        match target.file().filter(|path| super::supported(path)) {
            Some(path) => {
                scenes.insert(key, path);
                scenes.set_library(data_dir.join(LIBRARY_FILE));
                Ok(href(key, dark))
            }
            None => crate::browser::href(target).map_err(|error| error.to_string()),
        }
    }

    /// The public shape catalog that "Browse libraries" opens.
    const CATALOG: &str = "https://libraries.excalidraw.com/";

    pub(crate) fn catalog_url(url: &str) -> bool {
        url.starts_with(CATALOG)
    }

    /// The catalog's "Add to Excalidraw" link, which points back at a
    /// drawing page: the drawing's tab key, and a script that hands the
    /// library address to that page. Excalidraw downloads the library
    /// itself and asks the user unless the link carries the page's token.
    pub(crate) fn install_request(url: &str) -> Option<(String, String)> {
        let url = url::Url::parse(url)
            .ok()
            .filter(|url| own_url(url.as_str()))?;
        let key = url.query_pairs().find(|(name, _)| name == "id")?.1;
        let fragment = url.fragment()?;
        let (_, library) = url::form_urlencoded::parse(fragment.as_bytes())
            .find(|(name, _)| name == "addLibrary")?;
        if !catalog_url(&library) {
            return None;
        }
        let hash = serde_json::to_string(fragment).ok()?;
        Some((key.into_owned(), format!("location.hash = {hash};")))
    }

    /// A drawing view stays on the bundled page.
    pub(crate) fn own_url(url: &str) -> bool {
        url.strip_prefix(ORIGIN)
            .is_some_and(|rest| rest.starts_with('/'))
    }

    /// Scene file of each open drawing, by tab key. Shared with the scheme
    /// handler, which serves no path outside this map.
    #[derive(Clone, Default)]
    pub(crate) struct Scenes(Arc<Mutex<Open>>);

    #[derive(Default)]
    struct Open {
        files: HashMap<String, PathBuf>,
        /// The user's shape library, shared by all drawings.
        library: Option<PathBuf>,
        /// Scripts to run in drawing views, by tab key (library installs).
        scripts: Vec<(String, String)>,
    }

    impl Scenes {
        pub fn insert(&self, key: &str, path: &Path) {
            if let Ok(mut open) = self.0.lock() {
                open.files.insert(key.to_owned(), path.to_path_buf());
            }
        }

        pub fn set_library(&self, path: PathBuf) {
            if let Ok(mut open) = self.0.lock() {
                open.library = Some(path);
            }
        }

        pub fn push_script(&self, script: (String, String)) {
            if let Ok(mut open) = self.0.lock() {
                open.scripts.push(script);
            }
        }

        pub fn take_scripts(&self) -> Vec<(String, String)> {
            self.0
                .lock()
                .map(|mut open| std::mem::take(&mut open.scripts))
                .unwrap_or_default()
        }

        pub fn retain(&self, keep: impl Fn(&str) -> bool) {
            if let Ok(mut open) = self.0.lock() {
                open.files.retain(|key, _| keep(key));
            }
        }

        pub(super) fn path(&self, key: &str) -> Option<PathBuf> {
            self.0.lock().ok()?.files.get(key).cloned()
        }

        pub(super) fn library(&self) -> Option<PathBuf> {
            self.0.lock().ok()?.library.clone()
        }
    }

    /// Changes when the file changes: modified time and length.
    fn stamp(path: &Path) -> String {
        std::fs::metadata(path).map_or_else(
            |_| "none".into(),
            |meta| {
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |since| since.as_nanos());
                format!("{modified}-{}", meta.len())
            },
        )
    }

    /// `{stamp}` when the file still has stamp `known`, else `{stamp, text}`.
    /// A missing file is an empty drawing.
    pub(super) fn read_scene(path: &Path, known: &str) -> Result<String, String> {
        let stamp = stamp(path);
        if stamp == known {
            return Ok(serde_json::json!({ "stamp": stamp }).to_string());
        }
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error.to_string()),
        };
        Ok(serde_json::json!({ "stamp": stamp, "text": text }).to_string())
    }

    /// Replaces the scene file. Returns its new stamp.
    pub(super) fn write_scene(path: &Path, body: &[u8]) -> Result<String, String> {
        let scene: serde_json::Value =
            serde_json::from_slice(body).map_err(|error| error.to_string())?;
        if scene.get("type").and_then(serde_json::Value::as_str) != Some("excalidraw") {
            return Err("Not an Excalidraw scene".into());
        }
        terminator_core::atomic_replace(path, body).map_err(|error| error.to_string())?;
        Ok(stamp(path))
    }

    /// The library file text; empty when the user has no library yet.
    pub(super) fn read_library(path: &Path) -> Result<String, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(error) => Err(error.to_string()),
        }
    }

    pub(super) fn write_library(path: &Path, body: &[u8]) -> Result<String, String> {
        let library: serde_json::Value =
            serde_json::from_slice(body).map_err(|error| error.to_string())?;
        if library.get("type").and_then(serde_json::Value::as_str) != Some("excalidrawlib") {
            return Err("Not an Excalidraw library".into());
        }
        terminator_core::atomic_replace(path, body).map_err(|error| error.to_string())?;
        Ok(String::new())
    }

    /// Writes `<scene>.png` or `<scene>.svg` beside the scene file. Returns
    /// the file name.
    pub(super) fn write_image(path: &Path, body: &[u8], kind: &str) -> Result<String, String> {
        let valid = match kind {
            "png" => body.starts_with(b"\x89PNG\r\n\x1a\n"),
            _ => body.starts_with(b"<svg"),
        };
        if !valid {
            return Err(format!("Not a {kind} image"));
        }
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(format!(".{kind}"));
        let target = path.with_file_name(&name);
        terminator_core::atomic_replace(&target, body).map_err(|error| error.to_string())?;
        Ok(name.to_string_lossy().into_owned())
    }
}

#[cfg(feature = "webview")]
pub(crate) use scheme::serve;

#[cfg(feature = "webview")]
mod scheme {
    use super::page::{
        ORIGIN, Scenes, read_library, read_scene, write_image, write_library, write_scene,
    };
    use include_dir::{Dir, include_dir};
    use std::borrow::Cow;
    use wry::{
        RequestAsyncResponder,
        http::{Method, Request, Response, StatusCode, header},
    };

    static PAGE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../vendor/excalidraw/dist");

    type Body = Cow<'static, [u8]>;
    /// File work of one request: the reply text, or an error to show.
    type Work = fn(&std::path::Path, &Request<Vec<u8>>) -> Result<String, String>;

    fn reply(status: StatusCode, mime: &str, body: impl Into<Body>) -> Response<Body> {
        let mut response = Response::new(body.into());
        *response.status_mut() = status;
        if let Ok(mime) = header::HeaderValue::from_str(mime) {
            response.headers_mut().insert(header::CONTENT_TYPE, mime);
        }
        response
    }

    fn text(status: StatusCode, body: String) -> Response<Body> {
        reply(status, "text/plain; charset=utf-8", body.into_bytes())
    }

    fn mime(path: &str) -> &'static str {
        match path.rsplit('.').next().unwrap_or_default() {
            "html" => "text/html; charset=utf-8",
            "js" => "text/javascript; charset=utf-8",
            "css" => "text/css; charset=utf-8",
            "json" => "application/json",
            "woff2" => "font/woff2",
            "ttf" => "font/ttf",
            "png" => "image/png",
            "svg" => "image/svg+xml",
            "wasm" => "application/wasm",
            _ => "application/octet-stream",
        }
    }

    fn asset(path: &str) -> Response<Body> {
        PAGE.get_file(path.trim_start_matches('/')).map_or_else(
            || text(StatusCode::NOT_FOUND, "Not found".into()),
            |file| reply(StatusCode::OK, mime(path), file.contents()),
        )
    }

    fn query(request: &Request<Vec<u8>>, name: &str) -> String {
        url::form_urlencoded::parse(request.uri().query().unwrap_or_default().as_bytes())
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
            .unwrap_or_default()
    }

    /// Handler of the `terminator-draw` scheme. File work runs on its own
    /// thread, never on the GUI thread.
    pub(crate) fn serve(
        scenes: &Scenes,
        request: Request<Vec<u8>>,
        responder: RequestAsyncResponder,
    ) {
        let foreign = request
            .headers()
            .get(header::ORIGIN)
            .is_some_and(|origin| origin.as_bytes() != ORIGIN.as_bytes());
        if foreign {
            responder.respond(text(StatusCode::FORBIDDEN, "Forbidden".into()));
            return;
        }
        let route = (request.method().clone(), request.uri().path().to_owned());
        let work: Work = match (&route.0, route.1.as_str()) {
            (&Method::GET, "/scene") => |path, request| read_scene(path, &query(request, "stamp")),
            (&Method::POST, "/scene") => |path, request| write_scene(path, request.body()),
            (&Method::POST, "/png") => |path, request| write_image(path, request.body(), "png"),
            (&Method::POST, "/svg") => |path, request| write_image(path, request.body(), "svg"),
            (&Method::GET, "/library") => |path, _| read_library(path),
            (&Method::POST, "/library") => |path, request| write_library(path, request.body()),
            (&Method::GET, path) => {
                responder.respond(asset(path));
                return;
            }
            _ => {
                responder.respond(text(StatusCode::METHOD_NOT_ALLOWED, "Not allowed".into()));
                return;
            }
        };
        let path = if route.1 == "/library" {
            scenes.library()
        } else {
            scenes.path(&query(&request, "id"))
        };
        let Some(path) = path else {
            responder.respond(text(StatusCode::NOT_FOUND, "Drawing is not open".into()));
            return;
        };
        std::thread::spawn(move || {
            responder.respond(match work(&path, &request) {
                Ok(body) => text(StatusCode::OK, body),
                Err(error) => text(StatusCode::INTERNAL_SERVER_ERROR, error),
            });
        });
    }
}

#[cfg(test)]
#[path = "draw_tests.rs"]
mod tests;
