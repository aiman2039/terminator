//! Fallback (theme-tinted) icon names for files the colored language set does
//! not cover, grouped by icon.

use super::IconTable;

pub(super) const NAME_ICONS: IconTable = &[
    (
        "FileSliders",
        &[
            ".babelrc",
            ".dockerignore",
            ".editorconfig",
            ".eslintrc",
            ".eslintrc.cjs",
            ".eslintrc.js",
            ".eslintrc.yaml",
            ".eslintrc.yml",
            ".gitattributes",
            ".gitignore",
            ".npmrc",
            ".prettierrc",
            ".prettierrc.yaml",
            ".prettierrc.yml",
            "components.json",
            "postcss.config.cjs",
            "postcss.config.js",
            "postcss.config.mjs",
            "postcss.config.ts",
            "tailwind.config.cjs",
            "tailwind.config.js",
            "tailwind.config.mjs",
            "tailwind.config.ts",
            "tsconfig.json",
            "vite.config.js",
            "vite.config.mjs",
            "vite.config.ts",
            "vitest.config.js",
            "vitest.config.mjs",
            "vitest.config.ts",
        ],
    ),
    ("FileJson", &[".eslintrc.json", ".prettierrc.json"]),
    (
        "FileText",
        &[
            "agents.md",
            "authors",
            "changelog",
            "changelog.md",
            "contributing",
            "contributing.md",
            "readme",
            "readme.md",
            "todo",
        ],
    ),
    (
        "FileBox",
        &[
            "bun.lock",
            "bun.lockb",
            "cargo.lock",
            "cargo.toml",
            "composer.json",
            "composer.lock",
            "gemfile",
            "go.mod",
            "go.sum",
            "package-lock.json",
            "package.json",
            "pipfile",
            "pnpm-lock.yaml",
            "pnpm-workspace.yaml",
            "poetry.lock",
            "pom.xml",
            "pyproject.toml",
            "requirements-dev.txt",
            "requirements.txt",
            "yarn.lock",
        ],
    ),
    (
        "FileCog",
        &[
            "cmakelists.txt",
            "dockerfile",
            "meson.build",
            "settings.gradle",
            "settings.gradle.kts",
        ],
    ),
    ("FileKey", &["codeowners", "copying", "license", "notice"]),
    ("FileTerminal", &["makefile"]),
    ("FileLock", &["security", "security.md"]),
];

pub(super) const EXTENSION_ICONS: IconTable = &[
    (
        "FileArchive",
        &[
            "7z", "br", "bz2", "dmg", "gz", "iso", "rar", "tar", "tar.bz2", "tar.gz", "tar.xz",
            "tbz2", "tgz", "txz", "xz", "zip",
        ],
    ),
    (
        "FileMusic",
        &["aac", "flac", "m4a", "mp3", "ogg", "opus", "wav"],
    ),
    (
        "FileText",
        &[
            "adoc", "doc", "docx", "log", "md", "mdx", "pdf", "rst", "rtf", "tex", "txt",
        ],
    ),
    (
        "FileImage",
        &[
            "ai", "avif", "bmp", "eps", "gif", "heic", "ico", "jpeg", "jpg", "png", "psd", "svg",
            "tif", "tiff", "webp",
        ],
    ),
    (
        "FileKey",
        &["asc", "cer", "crt", "gpg", "key", "pem", "pub"],
    ),
    (
        "FileCode",
        &[
            "astro", "c", "cc", "cjs", "clj", "cpp", "cs", "cts", "cxx", "dart", "erl", "ex",
            "exs", "fs", "fsx", "go", "h", "hpp", "hrl", "hs", "htm", "html", "java", "js", "jsx",
            "kt", "kts", "lua", "mjs", "mts", "nim", "php", "pl", "pm", "py", "r", "rb", "rs",
            "scala", "sol", "svelte", "swift", "ts", "tsx", "vb", "vue", "xhtml", "xml", "zig",
        ],
    ),
    (
        "FileVideo",
        &["avi", "m4v", "mkv", "mov", "mp4", "mpeg", "mpg", "webm"],
    ),
    (
        "FileTerminal",
        &["bash", "bat", "cmd", "fish", "nu", "ps1", "sh", "zsh"],
    ),
    ("FileAxis3D", &["blend", "fbx", "glb", "gltf", "obj", "stl"]),
    (
        "FileSliders",
        &[
            "cfg",
            "conf",
            "hcl",
            "ini",
            "properties",
            "tf",
            "tfvars",
            "toml",
            "yaml",
            "yml",
        ],
    ),
    (
        "FileType",
        &[
            "css", "eot", "less", "otf", "sass", "scss", "ttf", "woff", "woff2",
        ],
    ),
    ("FileSpreadsheet", &["csv", "ods", "tsv", "xls", "xlsx"]),
    (
        "Database",
        &["db", "duckdb", "prisma", "sqlite", "sqlite3", "sql"],
    ),
    ("FileDiff", &["diff", "patch"]),
    ("FileBraces", &["gql", "graphql", "proto"]),
    ("FileCog", &["gradle"]),
    ("FileChartColumn", &["ipynb", "mmd", "ppt", "pptx"]),
    ("FileJson", &["json", "json5", "jsonc"]),
    ("FileLock", &["lock", "p12", "pfx"]),
];
