//! Colored per-language file glyphs from Material Icon Theme
//! (<https://github.com/material-extensions/vscode-material-icon-theme>, MIT,
//! see `../../assets/langs/LICENSE`). Names returned here are prefixed with
//! `lang:` so the renderer can skip theme tinting and keep the original colors.

use eframe::egui;

use super::{IconTables, lang_sources, lang_tables};

/// Icon for a lowercased file name, when the type is known.
pub(super) fn by_name(name: &str) -> Option<&'static str> {
    if name == ".env" || name.starts_with(".env.") {
        return Some("lang:key");
    }
    if name == "dockerfile" || name.starts_with("dockerfile.") || name == "containerfile" {
        return Some("lang:docker");
    }
    if name == "makefile" || name.starts_with("makefile.") {
        return Some("lang:makefile");
    }
    Some(match name {
        ".babelrc" | ".babelrc.json" => "lang:babel",
        ".dockerignore" => "lang:docker",
        ".editorconfig" => "lang:editorconfig",
        ".eslintrc" | ".eslintrc.cjs" | ".eslintrc.js" | ".eslintrc.json" | ".eslintrc.yaml"
        | ".eslintrc.yml" | "eslint.config.js" | "eslint.config.mjs" | "eslint.config.ts" => {
            "lang:eslint"
        }
        ".gitattributes" | ".gitignore" | ".gitmodules" | "codeowners" => "lang:git",
        ".npmrc" => "lang:npm",
        ".prettierignore"
        | ".prettierrc"
        | ".prettierrc.cjs"
        | ".prettierrc.json"
        | ".prettierrc.yaml"
        | ".prettierrc.yml"
        | "prettier.config.cjs"
        | "prettier.config.js"
        | "prettier.config.mjs"
        | "prettier.config.ts" => "lang:prettier",
        "agents.md" | "changelog" | "changelog.md" | "readme" | "readme.md" => "lang:markdown",
        "authors" | "contributing" | "contributing.md" | "security" | "security.md" | "todo"
        | "unlicense" => "lang:document",
        "bun.lock" | "bun.lockb" | "package-lock.json" | "package.json" => "lang:npm",
        "cargo.lock" | "cargo.toml" => "lang:rust",
        "cmakelists.txt" => "lang:cmake",
        "composer.json" | "composer.lock" => "lang:php",
        "copying" | "license" | "license.md" | "license.txt" | "notice" => "lang:license",
        "gemfile" | "rakefile" => "lang:gemfile",
        "go.mod" | "go.sum" => "lang:go-mod",
        "pipfile"
        | "pipfile.lock"
        | "poetry.lock"
        | "pyproject.toml"
        | "requirements.txt"
        | "requirements-dev.txt" => "lang:python",
        "pnpm-lock.yaml" | "pnpm-workspace.yaml" => "lang:pnpm",
        "pom.xml" => "lang:maven",
        "postcss.config.cjs" | "postcss.config.js" | "postcss.config.mjs" | "postcss.config.ts" => {
            "lang:postcss"
        }
        "build.gradle" | "build.gradle.kts" | "settings.gradle" | "settings.gradle.kts" => {
            "lang:gradle"
        }
        "tailwind.config.cjs"
        | "tailwind.config.js"
        | "tailwind.config.mjs"
        | "tailwind.config.ts" => "lang:tailwindcss",
        "tsconfig.json" => "lang:tsconfig",
        "vite.config.js" | "vite.config.mjs" | "vite.config.ts" | "vitest.config.js"
        | "vitest.config.mjs" | "vitest.config.ts" => "lang:vite",
        "yarn.lock" => "lang:yarn",
        _ => return None,
    })
}

/// Icon for a lowercased extension (multi-part tarball extensions included).
pub(super) fn by_extension(ext: &str) -> Option<&'static str> {
    IconTables::icon(lang_tables::EXTENSION_ICONS, ext)
}

/// Bundled colored SVG for a `lang:` name without the prefix.
pub(super) fn source(name: &str) -> egui::ImageSource<'static> {
    IconTables::source(lang_sources::LANG_SOURCES, name)
        .unwrap_or_else(|| egui::include_image!("../../assets/icons/file.svg"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_file_types_map_to_colored_icons() {
        assert_eq!(by_extension("rs"), Some("lang:rust"));
        assert_eq!(by_extension("sh"), Some("lang:console"));
        assert_eq!(by_extension("md"), Some("lang:markdown"));
        assert_eq!(by_name("dockerfile"), Some("lang:docker"));
        assert_eq!(by_name("cargo.toml"), Some("lang:rust"));
        assert_eq!(by_name("readme.md"), Some("lang:markdown"));
        assert_eq!(by_name("license"), Some("lang:license"));
        assert_eq!(by_name("unknown.zzz"), None);
        assert_eq!(by_extension("zzz"), None);
    }

    #[test]
    fn every_language_icon_resolves_to_a_bundled_svg() {
        for name in [
            "rust",
            "go",
            "python",
            "kotlin",
            "java",
            "console",
            "markdown",
            "docker",
            "toml",
            "yaml",
            "json",
            "typescript",
            "react_ts",
            "key",
            "license",
        ] {
            let _ = source(name);
        }
    }
}
