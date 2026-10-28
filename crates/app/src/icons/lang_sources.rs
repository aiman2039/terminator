//! Bundled colored language SVGs by `lang:` name (without the prefix).

use eframe::egui;

use super::SourceTable;

pub(super) static LANG_SOURCES: SourceTable = &[
    ("3d", egui::include_image!("../../assets/langs/3d.svg")),
    (
        "adobe-illustrator",
        egui::include_image!("../../assets/langs/adobe-illustrator.svg"),
    ),
    (
        "adobe-photoshop",
        egui::include_image!("../../assets/langs/adobe-photoshop.svg"),
    ),
    ("ada", egui::include_image!("../../assets/langs/ada.svg")),
    (
        "asciidoc",
        egui::include_image!("../../assets/langs/asciidoc.svg"),
    ),
    (
        "assembly",
        egui::include_image!("../../assets/langs/assembly.svg"),
    ),
    (
        "astro",
        egui::include_image!("../../assets/langs/astro.svg"),
    ),
    (
        "audio",
        egui::include_image!("../../assets/langs/audio.svg"),
    ),
    (
        "babel",
        egui::include_image!("../../assets/langs/babel.svg"),
    ),
    (
        "bibliography",
        egui::include_image!("../../assets/langs/bibliography.svg"),
    ),
    ("c", egui::include_image!("../../assets/langs/c.svg")),
    (
        "changelog",
        egui::include_image!("../../assets/langs/changelog.svg"),
    ),
    (
        "clojure",
        egui::include_image!("../../assets/langs/clojure.svg"),
    ),
    (
        "cmake",
        egui::include_image!("../../assets/langs/cmake.svg"),
    ),
    (
        "coffee",
        egui::include_image!("../../assets/langs/coffee.svg"),
    ),
    (
        "command",
        egui::include_image!("../../assets/langs/command.svg"),
    ),
    (
        "console",
        egui::include_image!("../../assets/langs/console.svg"),
    ),
    ("cpp", egui::include_image!("../../assets/langs/cpp.svg")),
    (
        "crystal",
        egui::include_image!("../../assets/langs/crystal.svg"),
    ),
    (
        "csharp",
        egui::include_image!("../../assets/langs/csharp.svg"),
    ),
    ("css", egui::include_image!("../../assets/langs/css.svg")),
    ("dart", egui::include_image!("../../assets/langs/dart.svg")),
    (
        "database",
        egui::include_image!("../../assets/langs/database.svg"),
    ),
    ("diff", egui::include_image!("../../assets/langs/diff.svg")),
    ("dll", egui::include_image!("../../assets/langs/dll.svg")),
    (
        "docker",
        egui::include_image!("../../assets/langs/docker.svg"),
    ),
    (
        "document",
        egui::include_image!("../../assets/langs/document.svg"),
    ),
    (
        "editorconfig",
        egui::include_image!("../../assets/langs/editorconfig.svg"),
    ),
    (
        "elixir",
        egui::include_image!("../../assets/langs/elixir.svg"),
    ),
    ("elm", egui::include_image!("../../assets/langs/elm.svg")),
    (
        "erlang",
        egui::include_image!("../../assets/langs/erlang.svg"),
    ),
    (
        "eslint",
        egui::include_image!("../../assets/langs/eslint.svg"),
    ),
    ("exe", egui::include_image!("../../assets/langs/exe.svg")),
    ("font", egui::include_image!("../../assets/langs/font.svg")),
    (
        "fortran",
        egui::include_image!("../../assets/langs/fortran.svg"),
    ),
    (
        "fsharp",
        egui::include_image!("../../assets/langs/fsharp.svg"),
    ),
    (
        "gemfile",
        egui::include_image!("../../assets/langs/gemfile.svg"),
    ),
    ("git", egui::include_image!("../../assets/langs/git.svg")),
    ("go", egui::include_image!("../../assets/langs/go.svg")),
    (
        "go-mod",
        egui::include_image!("../../assets/langs/go-mod.svg"),
    ),
    (
        "gradle",
        egui::include_image!("../../assets/langs/gradle.svg"),
    ),
    (
        "graphql",
        egui::include_image!("../../assets/langs/graphql.svg"),
    ),
    (
        "groovy",
        egui::include_image!("../../assets/langs/groovy.svg"),
    ),
    ("h", egui::include_image!("../../assets/langs/h.svg")),
    (
        "handlebars",
        egui::include_image!("../../assets/langs/handlebars.svg"),
    ),
    (
        "haskell",
        egui::include_image!("../../assets/langs/haskell.svg"),
    ),
    ("hcl", egui::include_image!("../../assets/langs/hcl.svg")),
    ("html", egui::include_image!("../../assets/langs/html.svg")),
    (
        "image",
        egui::include_image!("../../assets/langs/image.svg"),
    ),
    ("java", egui::include_image!("../../assets/langs/java.svg")),
    (
        "javascript",
        egui::include_image!("../../assets/langs/javascript.svg"),
    ),
    (
        "jinja",
        egui::include_image!("../../assets/langs/jinja.svg"),
    ),
    ("json", egui::include_image!("../../assets/langs/json.svg")),
    (
        "jupyter",
        egui::include_image!("../../assets/langs/jupyter.svg"),
    ),
    (
        "julia",
        egui::include_image!("../../assets/langs/julia.svg"),
    ),
    ("key", egui::include_image!("../../assets/langs/key.svg")),
    (
        "kotlin",
        egui::include_image!("../../assets/langs/kotlin.svg"),
    ),
    ("less", egui::include_image!("../../assets/langs/less.svg")),
    (
        "license",
        egui::include_image!("../../assets/langs/license.svg"),
    ),
    (
        "liquid",
        egui::include_image!("../../assets/langs/liquid.svg"),
    ),
    ("lisp", egui::include_image!("../../assets/langs/lisp.svg")),
    ("lock", egui::include_image!("../../assets/langs/lock.svg")),
    ("log", egui::include_image!("../../assets/langs/log.svg")),
    ("lua", egui::include_image!("../../assets/langs/lua.svg")),
    (
        "makefile",
        egui::include_image!("../../assets/langs/makefile.svg"),
    ),
    (
        "markdown",
        egui::include_image!("../../assets/langs/markdown.svg"),
    ),
    (
        "matlab",
        egui::include_image!("../../assets/langs/matlab.svg"),
    ),
    (
        "maven",
        egui::include_image!("../../assets/langs/maven.svg"),
    ),
    ("mdx", egui::include_image!("../../assets/langs/mdx.svg")),
    (
        "mermaid",
        egui::include_image!("../../assets/langs/mermaid.svg"),
    ),
    ("nim", egui::include_image!("../../assets/langs/nim.svg")),
    ("nix", egui::include_image!("../../assets/langs/nix.svg")),
    (
        "nodejs",
        egui::include_image!("../../assets/langs/nodejs.svg"),
    ),
    ("npm", egui::include_image!("../../assets/langs/npm.svg")),
    (
        "nunjucks",
        egui::include_image!("../../assets/langs/nunjucks.svg"),
    ),
    (
        "ocaml",
        egui::include_image!("../../assets/langs/ocaml.svg"),
    ),
    (
        "pascal",
        egui::include_image!("../../assets/langs/pascal.svg"),
    ),
    ("pdf", egui::include_image!("../../assets/langs/pdf.svg")),
    ("php", egui::include_image!("../../assets/langs/php.svg")),
    ("pnpm", egui::include_image!("../../assets/langs/pnpm.svg")),
    (
        "postcss",
        egui::include_image!("../../assets/langs/postcss.svg"),
    ),
    (
        "powerpoint",
        egui::include_image!("../../assets/langs/powerpoint.svg"),
    ),
    (
        "powershell",
        egui::include_image!("../../assets/langs/powershell.svg"),
    ),
    (
        "prettier",
        egui::include_image!("../../assets/langs/prettier.svg"),
    ),
    (
        "prisma",
        egui::include_image!("../../assets/langs/prisma.svg"),
    ),
    (
        "proto",
        egui::include_image!("../../assets/langs/proto.svg"),
    ),
    ("pug", egui::include_image!("../../assets/langs/pug.svg")),
    (
        "python",
        egui::include_image!("../../assets/langs/python.svg"),
    ),
    ("r", egui::include_image!("../../assets/langs/r.svg")),
    (
        "react",
        egui::include_image!("../../assets/langs/react.svg"),
    ),
    (
        "react_ts",
        egui::include_image!("../../assets/langs/react_ts.svg"),
    ),
    (
        "readme",
        egui::include_image!("../../assets/langs/readme.svg"),
    ),
    ("rust", egui::include_image!("../../assets/langs/rust.svg")),
    ("sass", egui::include_image!("../../assets/langs/sass.svg")),
    (
        "scala",
        egui::include_image!("../../assets/langs/scala.svg"),
    ),
    (
        "solidity",
        egui::include_image!("../../assets/langs/solidity.svg"),
    ),
    (
        "stylus",
        egui::include_image!("../../assets/langs/stylus.svg"),
    ),
    (
        "svelte",
        egui::include_image!("../../assets/langs/svelte.svg"),
    ),
    ("svg", egui::include_image!("../../assets/langs/svg.svg")),
    (
        "swift",
        egui::include_image!("../../assets/langs/swift.svg"),
    ),
    (
        "table",
        egui::include_image!("../../assets/langs/table.svg"),
    ),
    (
        "tailwindcss",
        egui::include_image!("../../assets/langs/tailwindcss.svg"),
    ),
    (
        "terraform",
        egui::include_image!("../../assets/langs/terraform.svg"),
    ),
    ("tex", egui::include_image!("../../assets/langs/tex.svg")),
    ("toml", egui::include_image!("../../assets/langs/toml.svg")),
    (
        "tsconfig",
        egui::include_image!("../../assets/langs/tsconfig.svg"),
    ),
    ("twig", egui::include_image!("../../assets/langs/twig.svg")),
    (
        "typescript",
        egui::include_image!("../../assets/langs/typescript.svg"),
    ),
    (
        "typst",
        egui::include_image!("../../assets/langs/typst.svg"),
    ),
    (
        "video",
        egui::include_image!("../../assets/langs/video.svg"),
    ),
    ("vim", egui::include_image!("../../assets/langs/vim.svg")),
    ("vite", egui::include_image!("../../assets/langs/vite.svg")),
    ("vue", egui::include_image!("../../assets/langs/vue.svg")),
    (
        "webassembly",
        egui::include_image!("../../assets/langs/webassembly.svg"),
    ),
    ("word", egui::include_image!("../../assets/langs/word.svg")),
    ("xml", egui::include_image!("../../assets/langs/xml.svg")),
    ("yaml", egui::include_image!("../../assets/langs/yaml.svg")),
    ("yarn", egui::include_image!("../../assets/langs/yarn.svg")),
    ("zig", egui::include_image!("../../assets/langs/zig.svg")),
    ("zip", egui::include_image!("../../assets/langs/zip.svg")),
];
