use super::*;

/// Workspace root for one file: nearest ancestor holding a language
/// marker (`Cargo.toml`, `pyproject.toml`, …), else nearest with `.git`,
/// else the file's own directory. Language markers win over a nearer
/// `.git` (a nested repo inside a buildable workspace still analyzes
/// with the workspace). Servers use the root for project-wide analysis.
pub(super) fn project_root(path: &Path, language: Language) -> PathBuf {
    let markers = root_markers(language);
    let mut fallback: Option<PathBuf> = None;
    let mut current = path.parent();
    for _ in 0..20 {
        let Some(dir) = current else {
            break;
        };
        if markers.iter().any(|name| dir.join(name).exists()) {
            return dir.to_path_buf();
        }
        if fallback.is_none() && dir.join(".git").exists() {
            fallback = Some(dir.to_path_buf());
        }
        current = dir.parent();
    }
    fallback.unwrap_or_else(|| path.parent().map(Path::to_path_buf).unwrap_or_default())
}

/// Files that mark a language's project root.
fn root_markers(language: Language) -> &'static [&'static str] {
    match language {
        Language::Rust => &["Cargo.toml"],
        Language::Python => &["pyproject.toml", "setup.py", "setup.cfg"],
        Language::TypeScript | Language::JavaScript => {
            &["tsconfig.json", "jsconfig.json", "package.json"]
        }
        Language::Markdown => &[".marksman.toml"],
        Language::Html | Language::Css | Language::Scss | Language::Less | Language::Json => {
            &["package.json"]
        }
        Language::Toml => &[".taplo.toml", "taplo.toml"],
        Language::Go => &["go.work", "go.mod"],
        Language::C | Language::Cpp => &[
            "compile_commands.json",
            "compile_flags.txt",
            ".clangd",
            "CMakeLists.txt",
        ],
        Language::Java => &[
            "pom.xml",
            "build.gradle",
            "build.gradle.kts",
            "settings.gradle",
        ],
        Language::Syntax("Lua") => &[".luarc.json", ".luarc.jsonc"],
        Language::Syntax("Ruby") => &["Gemfile"],
        Language::Syntax("PHP") => &["composer.json"],
        _ => &[],
    }
}
