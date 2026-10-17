Colored per-language file icons from [Material Icon Theme](https://github.com/material-extensions/vscode-material-icon-theme),
retrieved 2026-10-17. MIT licensed; see LICENSE.

These SVGs keep their own palette, so the renderer paints them without the
theme tint. `crates/app/src/icons/langs.rs` maps file names and extensions to
them and owns the bundled list; add an icon here and a `source` arm there to
extend coverage. Unknown types fall back to the tinted Lucide file icons in
`../icons/`.
