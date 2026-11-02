use eframe::egui;

mod file_tables;
mod lang_sources;
mod lang_tables;
mod langs;
mod ui_sources;

/// Icon name paired with every lowercased key (file name or extension) it covers.
type IconTable = &'static [(&'static str, &'static [&'static str])];
/// Bundled image per icon name.
type SourceTable = &'static [(&'static str, egui::ImageSource<'static>)];

/// Lookups over the static icon tables.
struct IconTables;

impl IconTables {
    fn icon(table: IconTable, key: &str) -> Option<&'static str> {
        table
            .iter()
            .find(|(_, keys)| keys.contains(&key))
            .map(|(icon, _)| *icon)
    }

    fn source(table: SourceTable, name: &str) -> Option<egui::ImageSource<'static>> {
        table
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, source)| source.clone())
    }

    /// Theme-tinted icon for file names matched by prefix or alias.
    fn by_name_pattern(name: &str) -> Option<&'static str> {
        if name == ".env" || name.starts_with(".env.") {
            return Some("FileLock");
        }
        if name.starts_with("dockerfile.") {
            return Some("FileCog");
        }
        if name.starts_with("makefile.") {
            return Some("FileTerminal");
        }
        if name == "mobile emulator" || name == "simulator" {
            return Some("Smartphone");
        }
        None
    }

    /// Lowercased extension, preferring multi-part tarball extensions.
    fn extension(name: &str) -> &str {
        ["tar.bz2", "tar.gz", "tar.xz"]
            .into_iter()
            .find(|ext| {
                name.strip_suffix(ext)
                    .is_some_and(|stem| stem.ends_with('.'))
            })
            .unwrap_or_else(|| {
                name.rsplit_once('.')
                    .filter(|(stem, _)| !stem.is_empty())
                    .map(|(_, ext)| ext)
                    .unwrap_or("")
            })
    }

    fn by_extension(name: &str) -> Option<&'static str> {
        let ext = Self::extension(name);
        langs::by_extension(ext).or_else(|| Self::icon(file_tables::EXTENSION_ICONS, ext))
    }
}

/// Colored, per-language glyph for files with a known type. These keep their
/// own colors; the renderer skips theme tinting for them.
pub fn is_language(icon: &str) -> bool {
    icon.starts_with("lang:")
}

/// Lowercased name, borrowed when it is already lowercase (most files),
/// so per-row icon lookups do not allocate every frame.
fn lowercase(name: std::borrow::Cow<'_, str>) -> std::borrow::Cow<'_, str> {
    if name.chars().any(char::is_uppercase) {
        std::borrow::Cow::Owned(name.to_lowercase())
    } else {
        name
    }
}

pub fn file_icon(path: &std::path::Path) -> &'static str {
    let name = lowercase(path.file_name().unwrap_or_default().to_string_lossy());
    langs::by_name(&name)
        .or_else(|| IconTables::icon(file_tables::NAME_ICONS, &name))
        .or_else(|| IconTables::by_name_pattern(&name))
        .or_else(|| IconTables::by_extension(&name))
        .unwrap_or("File")
}

/// `IntelliJ` project-tree role of a folder, guessed from common names:
/// a source root, a test root, or an excluded (ignored or build output)
/// folder. Each role has its own folder color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FolderRole {
    Source,
    Test,
    Excluded,
    Plain,
}

fn folder_name(path: &std::path::Path) -> std::borrow::Cow<'_, str> {
    lowercase(path.file_name().unwrap_or_default().to_string_lossy())
}

/// The folder's role in the project tree.
pub fn folder_role(path: &std::path::Path, ignored: bool) -> FolderRole {
    match &*folder_name(path) {
        _ if ignored => FolderRole::Excluded,
        "target" | "node_modules" | "dist" | "build" | "out" | ".venv" | "__pycache__" => {
            FolderRole::Excluded
        }
        "src" | "lib" | "source" => FolderRole::Source,
        "test" | "tests" | "spec" | "__tests__" | "e2e" => FolderRole::Test,
        _ => FolderRole::Plain,
    }
}

/// Folder icon by role (`IntelliJ` source/test/excluded roots, guessed from
/// common names): code, tests, Git metadata, build output or ignored, or a
/// plain open/closed folder.
pub fn folder_icon(path: &std::path::Path, open: bool, ignored: bool) -> &'static str {
    match folder_role(path, ignored) {
        FolderRole::Excluded => return "FolderX",
        FolderRole::Source => return "FolderCode",
        FolderRole::Test => return "FolderCheck",
        FolderRole::Plain => {}
    }
    match &*folder_name(path) {
        ".git" | ".github" | ".gitlab" => "FolderGit",
        "crates" | "packages" => "FolderCode",
        _ if open => "FolderOpen",
        _ => "Folder",
    }
}

pub fn source(name: &str) -> egui::ImageSource<'static> {
    if let Some(language) = name.strip_prefix("lang:") {
        return langs::source(language);
    }
    IconTables::source(ui_sources::UI_SOURCES, name)
        .unwrap_or(egui::include_image!("../../assets/icons/file.svg"))
}

/// `name` is in the shared UI icon table (no file-icon fallback).
#[cfg(test)]
pub fn is_ui_icon(name: &str) -> bool {
    IconTables::source(ui_sources::UI_SOURCES, name).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_icon_falls_back_through_name_and_extension_tables() {
        let icon = |name: &str| file_icon(std::path::Path::new(name));
        assert_eq!(icon("Cargo.toml"), "lang:rust");
        assert_eq!(icon("components.json"), "FileSliders");
        assert_eq!(icon("Simulator"), "Smartphone");
        assert_eq!(icon("backup.tar.gz"), "lang:zip");
        assert_eq!(icon("scene.blend"), "lang:3d");
        assert_eq!(icon("notes.zzz"), "File");
        assert_eq!(icon(".hidden"), "File");
        assert!(matches!(source("Play"), egui::ImageSource::Bytes { .. }));
    }

    #[test]
    fn lowercase_names_are_borrowed_and_tarball_needs_a_dot() {
        use std::borrow::Cow;
        assert!(matches!(
            lowercase(Cow::Borrowed("main.rs")),
            Cow::Borrowed(_)
        ));
        assert_eq!(lowercase(Cow::Borrowed("README.MD")), "readme.md");
        assert_eq!(IconTables::extension("backup.tar.gz"), "tar.gz");
        assert_eq!(IconTables::extension("backuptar.gz"), "gz");
    }

    #[test]
    fn folder_role_marks_source_test_and_excluded_roots() {
        let role = |name: &str, ignored| folder_role(std::path::Path::new(name), ignored);
        assert_eq!(role("repo/src", false), FolderRole::Source);
        assert_eq!(role("repo/Tests", false), FolderRole::Test);
        assert_eq!(role("repo/target", false), FolderRole::Excluded);
        assert_eq!(role("repo/src", true), FolderRole::Excluded);
        assert_eq!(role("repo/crates", false), FolderRole::Plain);
        assert_eq!(role("repo/docs", false), FolderRole::Plain);
    }

    #[test]
    fn folder_icon_shows_the_folder_role() {
        let icon =
            |name: &str, open, ignored| folder_icon(std::path::Path::new(name), open, ignored);
        assert_eq!(icon("repo/src", false, false), "FolderCode");
        assert_eq!(icon("repo/Tests", true, false), "FolderCheck");
        assert_eq!(icon("repo/.github", false, false), "FolderGit");
        assert_eq!(icon("repo/target", false, false), "FolderX");
        assert_eq!(icon("repo/src", false, true), "FolderX");
        assert_eq!(icon("repo/docs", true, false), "FolderOpen");
        assert_eq!(icon("repo/docs", false, false), "Folder");
        for name in [
            "FolderCode",
            "FolderCheck",
            "FolderGit",
            "FolderX",
            "Scissors",
        ] {
            assert!(IconTables::source(ui_sources::UI_SOURCES, name).is_some());
        }
    }

    #[test]
    fn grok_icon_decodes_with_application_loaders() {
        let ctx = egui::Context::default();
        crate::appearance::install(&ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let loaded = source("AgentGrok")
                .load(
                    &ctx,
                    egui::TextureOptions::LINEAR,
                    egui::SizeHint::Width(24),
                )
                .expect("bundled Grok PNG must have an installed decoder");
            if let egui::load::TexturePoll::Ready { texture } = loaded {
                assert!(texture.size.x > 0.0 && texture.size.y > 0.0);
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Grok decoder stayed pending"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
