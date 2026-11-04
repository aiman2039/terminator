//! Shared helpers for native buffer tests: load, save and pump the file
//! pool until a condition holds.
use super::*;

pub(super) fn app(dir: &Path) -> App {
    let ctx = egui::Context::default();
    App::with_context(&ctx, Paths::at(dir.join("state")))
}

/// Run loads (and their retries) until `done`, or fail after 10 s.
pub(super) fn pump(native: &mut NativeDoc, app: &App, done: impl Fn(&NativeDoc) -> bool) {
    let start = Instant::now();
    while !done(native) {
        assert!(start.elapsed() < Duration::from_secs(10), "timed out");
        native.ensure_loading(&app.services);
        native.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub(super) fn open(app: &App, path: &Path) -> NativeDoc {
    let mut native = NativeDoc::new(path.to_owned(), false);
    pump(&mut native, app, |native| native.doc.is_some());
    native
}

pub(super) fn save(native: &mut NativeDoc, app: &App) {
    native.start_save(&app.services);
    pump(native, app, |native| !native.saving);
}
