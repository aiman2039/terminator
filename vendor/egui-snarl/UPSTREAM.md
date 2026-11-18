# egui-snarl local patch

Vendored crates.io egui-snarl 0.12.0, retaining the upstream MIT and
Apache-2.0 license files. Upstream (also its `main` branch on 2026-11-18)
depends on egui 0.36; Terminator uses egui 0.37.

Changes:

- `Cargo.toml`: egui 0.36 becomes 0.37.
- `Cargo.toml`: the optional `egui-probe` dependency, the demo example and
  the dev-dependencies are removed. `egui-probe` 0.13 needs egui 0.36, so
  it would add a second egui to the lock file. The `cfg_attr(feature =
  "egui-probe", …)` lines stay in the source and are never active.
- The demo, the logo images and the CI files are not vendored.
- `SnarlViewer::menu_style` (new, default: no change): the style of the
  graph, node and dropped-wire context menus. The three menus use
  `egui::Popup::context_menu(..).style(..)`.
- egui 0.37 renames: `Ui::id` and `UiBuilder::id` become `scope_id`,
  `Id::new` becomes `Id::unique`, and `SnarlWidget::id_salt` takes
  `impl Hash + Debug` in place of the deprecated `egui::AsId`.

Terminator uses it for the native flow editor (`crates/app/src/flows/editor`).
