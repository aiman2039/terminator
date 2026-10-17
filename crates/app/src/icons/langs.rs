//! Colored per-language file glyphs from Material Icon Theme
//! (<https://github.com/material-extensions/vscode-material-icon-theme>, MIT,
//! see `../../assets/langs/LICENSE`). Names returned here are prefixed with
//! `lang:` so the renderer can skip theme tinting and keep the original colors.

use eframe::egui;

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
    Some(match ext {
        "7z" | "br" | "bz2" | "dmg" | "gz" | "iso" | "rar" | "tar" | "tar.bz2" | "tar.gz"
        | "tar.xz" | "tbz2" | "tgz" | "txz" | "xz" | "zip" => "lang:zip",
        "aac" | "flac" | "m4a" | "mp3" | "ogg" | "opus" | "wav" => "lang:audio",
        "avi" | "m4v" | "mkv" | "mov" | "mp4" | "mpeg" | "mpg" | "webm" => "lang:video",
        "avif" | "bmp" | "gif" | "heic" | "ico" | "jpeg" | "jpg" | "png" | "tif" | "tiff"
        | "webp" => "lang:image",
        "ai" => "lang:adobe-illustrator",
        "psd" => "lang:adobe-photoshop",
        "svg" => "lang:svg",
        "pdf" => "lang:pdf",
        "doc" | "docx" | "rtf" => "lang:word",
        "ppt" | "pptx" => "lang:powerpoint",
        "csv" | "ods" | "tsv" | "xls" | "xlsx" => "lang:table",
        "adoc" => "lang:asciidoc",
        "md" | "markdown" => "lang:markdown",
        "mdx" => "lang:mdx",
        "txt" => "lang:document",
        "rs" => "lang:rust",
        "go" => "lang:go",
        "py" | "pyi" | "pyw" => "lang:python",
        "java" => "lang:java",
        "kt" | "kts" => "lang:kotlin",
        "c" => "lang:c",
        "h" => "lang:h",
        "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" => "lang:cpp",
        "cs" => "lang:csharp",
        "cjs" | "js" | "mjs" => "lang:javascript",
        "jsx" => "lang:react",
        "cts" | "mts" | "ts" => "lang:typescript",
        "tsx" => "lang:react_ts",
        "vue" => "lang:vue",
        "svelte" => "lang:svelte",
        "astro" => "lang:astro",
        "htm" | "html" | "xhtml" => "lang:html",
        "css" => "lang:css",
        "sass" | "scss" => "lang:sass",
        "less" => "lang:less",
        "styl" => "lang:stylus",
        "json" | "json5" | "jsonc" => "lang:json",
        "yaml" | "yml" => "lang:yaml",
        "toml" => "lang:toml",
        "xml" => "lang:xml",
        "bash" | "fish" | "ksh" | "nu" | "sh" | "zsh" => "lang:console",
        "ps1" | "psm1" => "lang:powershell",
        "bat" | "cmd" => "lang:command",
        "lua" => "lang:lua",
        "php" => "lang:php",
        "rb" => "lang:ruby",
        "swift" => "lang:swift",
        "dart" => "lang:dart",
        "sc" | "scala" => "lang:scala",
        "groovy" => "lang:groovy",
        "hs" | "lhs" => "lang:haskell",
        "ex" | "exs" => "lang:elixir",
        "erl" | "hrl" => "lang:erlang",
        "clj" | "cljc" | "cljs" | "edn" => "lang:clojure",
        "nim" => "lang:nim",
        "zig" => "lang:zig",
        "nix" => "lang:nix",
        "prisma" => "lang:prisma",
        "gql" | "graphql" => "lang:graphql",
        "db" | "duckdb" | "sql" | "sqlite" | "sqlite3" => "lang:database",
        "proto" => "lang:proto",
        "tf" | "tfvars" => "lang:terraform",
        "hcl" => "lang:hcl",
        "cmake" => "lang:cmake",
        "mk" => "lang:makefile",
        "gradle" => "lang:gradle",
        "sol" => "lang:solidity",
        "r" | "rmd" => "lang:r",
        "jl" => "lang:julia",
        "ml" | "mli" => "lang:ocaml",
        "fs" | "fsi" | "fsx" => "lang:fsharp",
        "elm" => "lang:elm",
        "cr" => "lang:crystal",
        "wasm" | "wat" => "lang:webassembly",
        "asm" | "s" => "lang:assembly",
        "vim" => "lang:vim",
        "cl" | "el" | "lisp" | "lsp" => "lang:lisp",
        "f" | "f90" | "f95" | "for" => "lang:fortran",
        "ada" | "adb" | "ads" => "lang:ada",
        "pas" | "pp" => "lang:pascal",
        "coffee" => "lang:coffee",
        "jade" | "pug" => "lang:pug",
        "handlebars" | "hbs" => "lang:handlebars",
        "twig" => "lang:twig",
        "liquid" => "lang:liquid",
        "j2" | "jinja" => "lang:jinja",
        "njk" => "lang:nunjucks",
        "mmd" => "lang:mermaid",
        "typ" => "lang:typst",
        "ipynb" => "lang:jupyter",
        "tex" => "lang:tex",
        "bib" => "lang:bibliography",
        "lock" => "lang:lock",
        "asc" | "cer" | "crt" | "gpg" | "key" | "p12" | "pem" | "pfx" | "pub" => "lang:key",
        "eot" | "otf" | "ttf" | "woff" | "woff2" => "lang:font",
        "diff" | "patch" => "lang:diff",
        "exe" => "lang:exe",
        "dll" | "dylib" | "so" => "lang:dll",
        "blend" | "fbx" | "glb" | "gltf" | "obj" | "stl" => "lang:3d",
        "log" => "lang:log",
        _ => return None,
    })
}

/// Bundled colored SVG for a `lang:` name without the prefix.
pub(super) fn source(name: &str) -> egui::ImageSource<'static> {
    match name {
        "3d" => egui::include_image!("../../assets/langs/3d.svg"),
        "adobe-illustrator" => egui::include_image!("../../assets/langs/adobe-illustrator.svg"),
        "adobe-photoshop" => egui::include_image!("../../assets/langs/adobe-photoshop.svg"),
        "ada" => egui::include_image!("../../assets/langs/ada.svg"),
        "asciidoc" => egui::include_image!("../../assets/langs/asciidoc.svg"),
        "assembly" => egui::include_image!("../../assets/langs/assembly.svg"),
        "astro" => egui::include_image!("../../assets/langs/astro.svg"),
        "audio" => egui::include_image!("../../assets/langs/audio.svg"),
        "babel" => egui::include_image!("../../assets/langs/babel.svg"),
        "bibliography" => egui::include_image!("../../assets/langs/bibliography.svg"),
        "c" => egui::include_image!("../../assets/langs/c.svg"),
        "changelog" => egui::include_image!("../../assets/langs/changelog.svg"),
        "clojure" => egui::include_image!("../../assets/langs/clojure.svg"),
        "cmake" => egui::include_image!("../../assets/langs/cmake.svg"),
        "coffee" => egui::include_image!("../../assets/langs/coffee.svg"),
        "command" => egui::include_image!("../../assets/langs/command.svg"),
        "console" => egui::include_image!("../../assets/langs/console.svg"),
        "cpp" => egui::include_image!("../../assets/langs/cpp.svg"),
        "crystal" => egui::include_image!("../../assets/langs/crystal.svg"),
        "csharp" => egui::include_image!("../../assets/langs/csharp.svg"),
        "css" => egui::include_image!("../../assets/langs/css.svg"),
        "dart" => egui::include_image!("../../assets/langs/dart.svg"),
        "database" => egui::include_image!("../../assets/langs/database.svg"),
        "diff" => egui::include_image!("../../assets/langs/diff.svg"),
        "dll" => egui::include_image!("../../assets/langs/dll.svg"),
        "docker" => egui::include_image!("../../assets/langs/docker.svg"),
        "document" => egui::include_image!("../../assets/langs/document.svg"),
        "editorconfig" => egui::include_image!("../../assets/langs/editorconfig.svg"),
        "elixir" => egui::include_image!("../../assets/langs/elixir.svg"),
        "elm" => egui::include_image!("../../assets/langs/elm.svg"),
        "erlang" => egui::include_image!("../../assets/langs/erlang.svg"),
        "eslint" => egui::include_image!("../../assets/langs/eslint.svg"),
        "exe" => egui::include_image!("../../assets/langs/exe.svg"),
        "font" => egui::include_image!("../../assets/langs/font.svg"),
        "fortran" => egui::include_image!("../../assets/langs/fortran.svg"),
        "fsharp" => egui::include_image!("../../assets/langs/fsharp.svg"),
        "gemfile" => egui::include_image!("../../assets/langs/gemfile.svg"),
        "git" => egui::include_image!("../../assets/langs/git.svg"),
        "go" => egui::include_image!("../../assets/langs/go.svg"),
        "go-mod" => egui::include_image!("../../assets/langs/go-mod.svg"),
        "gradle" => egui::include_image!("../../assets/langs/gradle.svg"),
        "graphql" => egui::include_image!("../../assets/langs/graphql.svg"),
        "groovy" => egui::include_image!("../../assets/langs/groovy.svg"),
        "h" => egui::include_image!("../../assets/langs/h.svg"),
        "handlebars" => egui::include_image!("../../assets/langs/handlebars.svg"),
        "haskell" => egui::include_image!("../../assets/langs/haskell.svg"),
        "hcl" => egui::include_image!("../../assets/langs/hcl.svg"),
        "html" => egui::include_image!("../../assets/langs/html.svg"),
        "image" => egui::include_image!("../../assets/langs/image.svg"),
        "java" => egui::include_image!("../../assets/langs/java.svg"),
        "javascript" => egui::include_image!("../../assets/langs/javascript.svg"),
        "jinja" => egui::include_image!("../../assets/langs/jinja.svg"),
        "json" => egui::include_image!("../../assets/langs/json.svg"),
        "jupyter" => egui::include_image!("../../assets/langs/jupyter.svg"),
        "julia" => egui::include_image!("../../assets/langs/julia.svg"),
        "key" => egui::include_image!("../../assets/langs/key.svg"),
        "kotlin" => egui::include_image!("../../assets/langs/kotlin.svg"),
        "less" => egui::include_image!("../../assets/langs/less.svg"),
        "license" => egui::include_image!("../../assets/langs/license.svg"),
        "liquid" => egui::include_image!("../../assets/langs/liquid.svg"),
        "lisp" => egui::include_image!("../../assets/langs/lisp.svg"),
        "lock" => egui::include_image!("../../assets/langs/lock.svg"),
        "log" => egui::include_image!("../../assets/langs/log.svg"),
        "lua" => egui::include_image!("../../assets/langs/lua.svg"),
        "makefile" => egui::include_image!("../../assets/langs/makefile.svg"),
        "markdown" => egui::include_image!("../../assets/langs/markdown.svg"),
        "matlab" => egui::include_image!("../../assets/langs/matlab.svg"),
        "maven" => egui::include_image!("../../assets/langs/maven.svg"),
        "mdx" => egui::include_image!("../../assets/langs/mdx.svg"),
        "mermaid" => egui::include_image!("../../assets/langs/mermaid.svg"),
        "nim" => egui::include_image!("../../assets/langs/nim.svg"),
        "nix" => egui::include_image!("../../assets/langs/nix.svg"),
        "nodejs" => egui::include_image!("../../assets/langs/nodejs.svg"),
        "npm" => egui::include_image!("../../assets/langs/npm.svg"),
        "nunjucks" => egui::include_image!("../../assets/langs/nunjucks.svg"),
        "ocaml" => egui::include_image!("../../assets/langs/ocaml.svg"),
        "pascal" => egui::include_image!("../../assets/langs/pascal.svg"),
        "pdf" => egui::include_image!("../../assets/langs/pdf.svg"),
        "php" => egui::include_image!("../../assets/langs/php.svg"),
        "pnpm" => egui::include_image!("../../assets/langs/pnpm.svg"),
        "postcss" => egui::include_image!("../../assets/langs/postcss.svg"),
        "powerpoint" => egui::include_image!("../../assets/langs/powerpoint.svg"),
        "powershell" => egui::include_image!("../../assets/langs/powershell.svg"),
        "prettier" => egui::include_image!("../../assets/langs/prettier.svg"),
        "prisma" => egui::include_image!("../../assets/langs/prisma.svg"),
        "proto" => egui::include_image!("../../assets/langs/proto.svg"),
        "pug" => egui::include_image!("../../assets/langs/pug.svg"),
        "python" => egui::include_image!("../../assets/langs/python.svg"),
        "r" => egui::include_image!("../../assets/langs/r.svg"),
        "react" => egui::include_image!("../../assets/langs/react.svg"),
        "react_ts" => egui::include_image!("../../assets/langs/react_ts.svg"),
        "readme" => egui::include_image!("../../assets/langs/readme.svg"),
        "rust" => egui::include_image!("../../assets/langs/rust.svg"),
        "sass" => egui::include_image!("../../assets/langs/sass.svg"),
        "scala" => egui::include_image!("../../assets/langs/scala.svg"),
        "solidity" => egui::include_image!("../../assets/langs/solidity.svg"),
        "stylus" => egui::include_image!("../../assets/langs/stylus.svg"),
        "svelte" => egui::include_image!("../../assets/langs/svelte.svg"),
        "svg" => egui::include_image!("../../assets/langs/svg.svg"),
        "swift" => egui::include_image!("../../assets/langs/swift.svg"),
        "table" => egui::include_image!("../../assets/langs/table.svg"),
        "tailwindcss" => egui::include_image!("../../assets/langs/tailwindcss.svg"),
        "terraform" => egui::include_image!("../../assets/langs/terraform.svg"),
        "tex" => egui::include_image!("../../assets/langs/tex.svg"),
        "toml" => egui::include_image!("../../assets/langs/toml.svg"),
        "tsconfig" => egui::include_image!("../../assets/langs/tsconfig.svg"),
        "twig" => egui::include_image!("../../assets/langs/twig.svg"),
        "typescript" => egui::include_image!("../../assets/langs/typescript.svg"),
        "typst" => egui::include_image!("../../assets/langs/typst.svg"),
        "video" => egui::include_image!("../../assets/langs/video.svg"),
        "vim" => egui::include_image!("../../assets/langs/vim.svg"),
        "vite" => egui::include_image!("../../assets/langs/vite.svg"),
        "vue" => egui::include_image!("../../assets/langs/vue.svg"),
        "webassembly" => egui::include_image!("../../assets/langs/webassembly.svg"),
        "word" => egui::include_image!("../../assets/langs/word.svg"),
        "xml" => egui::include_image!("../../assets/langs/xml.svg"),
        "yaml" => egui::include_image!("../../assets/langs/yaml.svg"),
        "yarn" => egui::include_image!("../../assets/langs/yarn.svg"),
        "zig" => egui::include_image!("../../assets/langs/zig.svg"),
        "zip" => egui::include_image!("../../assets/langs/zip.svg"),
        _ => egui::include_image!("../../assets/icons/file.svg"),
    }
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
