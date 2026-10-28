use super::*;

impl App {
    pub(super) fn project_for_palette_path(&self, path: &Path) -> Option<&Project> {
        self.state
            .projects
            .iter()
            .filter(|project| {
                self.project_available(&project.id) && path.starts_with(&project.path)
            })
            .max_by_key(|project| project.path.components().count())
    }

    pub(crate) fn palette_items(&self) -> Vec<PaletteItem> {
        let mut items = self.waiting_agent_items();
        self.push_project_and_session_items(&mut items);
        self.push_symbol_items(&mut items);
        self.push_command_items(&mut items);
        self.push_fallback_file_items(&mut items);
        items
    }

    /// Agents waiting on input or permission, newest first, one per session.
    fn waiting_agent_items(&self) -> Vec<PaletteItem> {
        let mut items = Vec::new();
        let mut notices: Vec<_> = self
            .state
            .notifications
            .iter()
            .filter(|notice| {
                matches!(
                    notice.state,
                    AgentState::WaitingInput | AgentState::WaitingPermission
                ) && agent_presence::notice_pending(notice, now())
            })
            .collect();
        notices.sort_by_key(|notice| std::cmp::Reverse(notice.created));
        let mut seen_agents = HashSet::new();
        for notice in notices {
            let Some(session) = self
                .state
                .sessions
                .iter()
                .find(|session| session.id == notice.session_id && session.lifecycle.live())
            else {
                continue;
            };
            if !seen_agents.insert(&session.id) {
                continue;
            }
            let summary: String = notice
                .summary
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(80)
                .collect();
            items.push(PaletteItem::Agent(
                session.id.clone(),
                session.label.clone(),
                summary,
            ));
        }
        items
    }

    fn push_project_and_session_items(&self, items: &mut Vec<PaletteItem>) {
        for project in self.visible_projects() {
            items.push(PaletteItem::Project(
                project.id.clone(),
                project.name.clone(),
            ));
        }
        for session in self.state.sessions.iter().filter(|s| s.lifecycle.live()) {
            items.push(PaletteItem::Session(
                session.id.clone(),
                session.label.clone(),
            ));
        }
    }

    /// Open documents' symbols, then workspace hits not already listed.
    fn push_symbol_items(&self, items: &mut Vec<PaletteItem>) {
        for (path, symbol) in self.lsp.all_document_symbols() {
            if self.project_for_palette_path(&path).is_some() {
                items.push(PaletteItem::Symbol {
                    path,
                    name: symbol.name,
                    kind: symbol.kind,
                    line: symbol.selection.start.line,
                    column: symbol.selection.start.character,
                    container: None,
                });
            }
        }
        let mut seen_symbols: HashSet<(PathBuf, String, u32)> = items
            .iter()
            .filter_map(|item| match item {
                PaletteItem::Symbol {
                    path, name, line, ..
                } => Some((path.clone(), name.clone(), *line)),
                _ => None,
            })
            .collect();
        for hit in self.lsp.workspace_symbols() {
            let Some(path) = native_lsp::uri_to_path(&hit.uri) else {
                continue;
            };
            if self.project_for_palette_path(&path).is_some()
                && seen_symbols.insert((path.clone(), hit.name.clone(), hit.range.start.line))
            {
                items.push(PaletteItem::Symbol {
                    path,
                    name: hit.name,
                    kind: hit.kind,
                    line: hit.range.start.line,
                    column: hit.range.start.character,
                    container: hit.container,
                });
            }
        }
    }

    /// Settings, pinned and remaining actions, VNC hosts, and layouts.
    fn push_command_items(&self, items: &mut Vec<PaletteItem>) {
        for section in SettingsSection::ALL {
            items.push(PaletteItem::Settings(section));
        }
        items.push(PaletteItem::AddProject);
        items.extend(PINNED_ACTIONS.iter().copied().map(PaletteItem::Action));
        items.push(PaletteItem::OpenPlayer);
        items.extend(PINNED_IDE_ACTIONS.iter().copied().map(PaletteItem::Action));
        items.extend([PaletteItem::ToggleIdeSidebarHeight, PaletteItem::SaveLayout]);
        for profile in &self.state.vnc_profiles {
            items.push(PaletteItem::VncProfile(
                profile.id.clone(),
                profile.display_name(),
            ));
        }
        items.extend(
            Action::ALL
                .iter()
                .copied()
                .filter(|&action| self.palette_lists_action(action))
                .map(PaletteItem::Action),
        );
        for name in self.preferences.named_layouts.keys() {
            items.push(PaletteItem::ApplyLayout(name.clone()));
        }
        if self
            .state
            .capabilities
            .iter()
            .any(|capability| capability == WORKTREES_CAPABILITY)
        {
            items.push(PaletteItem::NewWorktree);
        }
    }

    fn palette_lists_action(&self, action: Action) -> bool {
        !pinned_action(action)
            && !HIDDEN_ACTIONS.contains(&action)
            && self.shortcut_applies(action)
            && (!SESSION_ACTIONS.contains(&action) || self.active_session.is_some())
    }

    /// Before the file index is ready: files from loaded directories and
    /// the Git context.
    fn push_fallback_file_items(&self, items: &mut Vec<PaletteItem>) {
        if !self.file_index.is_empty() {
            return;
        }
        let mut files: Vec<_> = self
            .dirs
            .values()
            .flatten()
            .filter(|entry| !entry.directory)
            .map(|entry| entry.path.clone())
            .collect();
        if let Some(context) = &self.context {
            files.extend(context.changes.iter().map(|change| change.path.clone()));
        }
        files.sort();
        files.dedup();
        items.extend(files.into_iter().map(PaletteItem::File));
    }
}
