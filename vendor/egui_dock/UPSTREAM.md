# egui_dock local patch

Vendored crates.io egui_dock 0.21.1, retaining upstream MIT license.
Adds default no-op `trailing_controls_width` and `trailing_controls` TabViewer callbacks.
The leaf reserves their width alongside the add button before clipping/scrolling tabs,
and renders the controls adjacent to it. Existing docking and scroll code is retained.

The flatter UI adds a default-true `show_tab_bar(NodePath)` callback. Terminator
uses split ancestry to hide headers below a vertical split's top edge. Policy-hidden
panes reserve no header height and do not show the upstream header-reveal control.
Tabs and focus remain in the dock state; lower-pane tab selection and creation are
available through the application context menu.

Leading tab icons add default `tab_leading_width` (0.0) and `paint_tab_leading`
(no-op) TabViewer callbacks. `tab_title` reserves the width before the title,
shifts the title right, and returns the icon rect; the leaf paints it through
the viewer after each tab button (including the drag ghost). Terminator paints
the agent brand plus hook lifecycle status there so native strip tabs match
the workspace strip and split-pane captions.
