//! Explorer tree keyboard: navigation, file operations, and type-to-search.
use super::*;

/// The visible rows and the selection cursor among them for one key pass.
struct TreeNav<'a> {
    rows: &'a [ExplorerRow],
    visible: &'a [&'a ExplorerRow],
    /// Index of the cursor row in `visible`.
    at: Option<usize>,
    cursor: Option<&'a PathBuf>,
    root: &'a Path,
}

impl TreeNav<'_> {
    fn current(&self) -> Option<&ExplorerRow> {
        self.at.and_then(|i| self.visible.get(i)).copied()
    }
}

impl App {
    /// Keyboard for a focused tree. Arrows move the selection (Shift
    /// extends), Left/Right collapse, expand, and walk the hierarchy,
    /// Enter opens, F2 renames, Delete trashes, Cmd/Ctrl+C/X/V copy, cut,
    /// and paste files, Escape clears, and typed text searches the
    /// visible rows.
    pub(in crate::sidebar_ui) fn explorer_keys(&mut self, input: ExplorerKeysInput<'_>) {
        let ExplorerKeysInput {
            ui,
            rows,
            host,
            root,
        } = input;
        let focus = tree_focus_id(host);
        if !ui.memory(|memory| memory.has_focus(focus)) || self.name_prompt.is_some() {
            return;
        }
        Self::lock_tree_keys(ui, focus);
        let (keys, typed) = Self::tree_key_events(ui);
        let visible = entry_rows(rows);
        let cursor = self.explorer_selection.last().cloned();
        let at = cursor
            .as_ref()
            .and_then(|cursor| visible.iter().position(|row| &row.path == cursor));
        let nav = TreeNav {
            rows,
            visible: &visible,
            at,
            cursor: cursor.as_ref(),
            root,
        };
        let mut consumed: Vec<(egui::Key, egui::Modifiers)> = Vec::new();
        for (key, modifiers) in keys {
            if self.explorer_key(ui, &nav, (key, modifiers)) {
                consumed.push((key, modifiers));
            }
        }
        self.explorer_type_to_search(ui, &typed, &nav);
        ui.input_mut(|input| {
            for (key, modifiers) in consumed {
                input.consume_key(modifiers, key);
            }
        });
    }

    /// Keep arrows and Escape on the tree instead of moving focus.
    fn lock_tree_keys(ui: &egui::Ui, focus: egui::Id) {
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                focus,
                egui::EventFilter {
                    tab: false,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            );
        });
    }

    /// Pressed keys and typed text this frame.
    fn tree_key_events(ui: &egui::Ui) -> (Vec<(egui::Key, egui::Modifiers)>, String) {
        let mut keys: Vec<(egui::Key, egui::Modifiers)> = Vec::new();
        let mut typed = String::new();
        ui.input(|input| {
            for event in &input.events {
                match event {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => keys.push((*key, *modifiers)),
                    egui::Event::Text(text) => typed.push_str(text),
                    _ => {}
                }
            }
        });
        (keys, typed)
    }

    /// Act on one key. Returns true when the tree handled it.
    fn explorer_key(
        &mut self,
        ui: &mut egui::Ui,
        nav: &TreeNav<'_>,
        (key, modifiers): (egui::Key, egui::Modifiers),
    ) -> bool {
        match key {
            egui::Key::ArrowDown | egui::Key::ArrowUp if !modifiers.command => {
                self.explorer_key_vertical(nav, key, modifiers.shift);
            }
            egui::Key::ArrowLeft => self.explorer_key_left(nav),
            egui::Key::ArrowRight => self.explorer_key_right(nav),
            egui::Key::Enter => self.explorer_key_enter(ui, nav),
            egui::Key::F2 | egui::Key::F6 if key == egui::Key::F2 || modifiers.shift => {
                if let Some(path) = nav.cursor {
                    self.explorer_rename(path);
                }
            }
            egui::Key::Delete => self.explorer_confirm_delete(),
            egui::Key::Backspace if modifiers.command => self.explorer_confirm_delete(),
            egui::Key::C | egui::Key::X if modifiers.command && !modifiers.alt => {
                self.explorer_copy_selection(ui, key == egui::Key::X);
            }
            egui::Key::V if modifiers.command && !modifiers.alt => self.explorer_paste(nav.root),
            egui::Key::Escape => {
                if self.explorer_speed.is_empty() {
                    self.explorer_selection.clear();
                } else {
                    self.explorer_speed.clear();
                }
            }
            egui::Key::Backspace if !self.explorer_speed.is_empty() => {
                self.explorer_speed.pop();
            }
            _ => return false,
        }
        true
    }

    /// Up/Down move the cursor; with Shift they extend from the anchor.
    fn explorer_key_vertical(&mut self, nav: &TreeNav<'_>, key: egui::Key, shift: bool) {
        let next = match (nav.at, key) {
            (None, _) => Some(0),
            (Some(i), egui::Key::ArrowDown) => {
                Some(i.saturating_add(1).min(nav.visible.len().saturating_sub(1)))
            }
            (Some(i), _) => Some(i.saturating_sub(1)),
        };
        let Some(row) = next.and_then(|next| nav.visible.get(next)) else {
            return;
        };
        let path = row.path.clone();
        if shift && let Some(anchor) = self.explorer_anchor.clone() {
            self.explorer_select_range(nav.rows, &anchor, &path);
            self.explorer_scroll_to = Some(path);
        } else {
            self.explorer_select_one(path);
        }
    }

    /// Left collapses an open folder, else selects the parent.
    fn explorer_key_left(&mut self, nav: &TreeNav<'_>) {
        let Some(row) = nav.current() else {
            return;
        };
        if is_dir(row) && self.explorer_row_expanded(row) {
            self.set_explorer_row_expanded(row, false);
        } else if let Some(parent) = row.chain.first().unwrap_or(&row.path).parent()
            && nav.visible.iter().any(|row| row.path == parent)
        {
            self.explorer_select_one(parent.to_owned());
        }
    }

    /// Right expands a closed folder, else selects its first child.
    fn explorer_key_right(&mut self, nav: &TreeNav<'_>) {
        let Some(row) = nav.current().filter(|row| is_dir(row)) else {
            return;
        };
        if !self.explorer_row_expanded(row) {
            self.set_explorer_row_expanded(row, true);
        } else if let Some(child) = nav
            .at
            .and_then(|i| nav.visible.get(i.saturating_add(1)))
            .filter(|child| child.path.parent() == Some(row.path.as_path()))
        {
            self.explorer_select_one(child.path.clone());
        }
    }

    /// Enter toggles a folder or opens a file.
    fn explorer_key_enter(&mut self, ui: &mut egui::Ui, nav: &TreeNav<'_>) {
        let Some(row) = nav.current() else {
            return;
        };
        if is_dir(row) {
            let open = self.explorer_row_expanded(row);
            self.set_explorer_row_expanded(row, !open);
        } else {
            let path = row.path.clone();
            self.activate_file_action(ui, &path, FileAction::Open);
        }
    }

    /// Copy (or cut) the selected paths, also as text.
    fn explorer_copy_selection(&mut self, ui: &egui::Ui, cut: bool) {
        if self.explorer_selection.is_empty() {
            return;
        }
        let paths = self.explorer_selection.clone();
        ui.ctx().copy_text(
            paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        );
        self.explorer_clipboard = Some((paths, cut));
    }

    /// Typed text extends the speed search (reset after idle) and jumps.
    fn explorer_type_to_search(&mut self, ui: &egui::Ui, typed: &str, nav: &TreeNav<'_>) {
        let typed: String = typed.chars().filter(|c| !c.is_control()).collect();
        if typed.is_empty() {
            return;
        }
        if self.explorer_speed_at.elapsed() > SPEED_SEARCH_IDLE {
            self.explorer_speed.clear();
        }
        self.explorer_speed.push_str(&typed);
        self.explorer_speed_at = Instant::now();
        self.explorer_speed_jump(nav.visible, nav.at);
        ui.input_mut(|input| {
            input
                .events
                .retain(|event| !matches!(event, egui::Event::Text(_)));
        });
    }
}
