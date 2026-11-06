//! Add to agent context: an `@path#L…` reference pasted into the
//! project's live agent terminal.
use super::*;

impl App {
    /// Add to agent context: paste an `@path#Lfirst-last` reference into
    /// the project's live agent terminal (the focused one, else the
    /// newest). Bracketed paste, no Enter: the user sends it.
    pub(crate) fn add_to_agent_context(
        &mut self,
        path: &Path,
        project: Option<&str>,
        (first, last): (usize, usize),
    ) {
        let project = project
            .map(str::to_owned)
            .or_else(|| self.selected_project().map(|p| p.id.clone()));
        let root = project.as_deref().and_then(|id| {
            self.state
                .projects
                .iter()
                .find(|p| p.id == id)
                .map(|p| p.path.clone())
        });
        let target = self
            .state
            .sessions
            .iter()
            .filter(|session| {
                Some(&session.project_id) == project.as_ref()
                    && session.lifecycle.live()
                    && self.backends.contains_key(&session.id)
                    && self.present_session(&session.id).live
            })
            .max_by_key(|session| {
                (
                    self.active_session.as_ref() == Some(&session.id),
                    session.created,
                )
            })
            .map(|session| (session.id.clone(), session.label.clone()));
        let Some((session, label)) = target else {
            self.info = Some("No open agent terminal in this project".into());
            return;
        };
        let reference = agent_reference(path, root.as_deref(), first, last);
        self.handle_clipboard_paste(&session, &format!("{reference} "));
        self.info = Some(format!("Added {reference} to {label}"));
    }
}

/// `@path#L4` or `@path#L4-9`, the path relative to the project root.
pub(crate) fn agent_reference(
    path: &Path,
    root: Option<&Path>,
    first: usize,
    last: usize,
) -> String {
    let shown = root
        .and_then(|root| path.strip_prefix(root).ok())
        .unwrap_or(path);
    let lines = if first == last {
        format!("L{first}")
    } else {
        format!("L{first}-{last}")
    };
    format!("@{}#{lines}", shown.display())
}

impl NativeDoc {
    /// First and last selected line, 1-based; the caret line without a
    /// selection.
    pub(crate) fn selected_lines(&self) -> (usize, usize) {
        let caret = self.engine.cursor();
        let (from, to) = self.engine.selection().unwrap_or((caret, caret));
        let (first, last) = (from.line.min(to.line), from.line.max(to.line));
        (first.saturating_add(1), last.saturating_add(1))
    }
}

impl App {
    /// `@path#L…` for the focused native editor's selection, if any.
    pub(crate) fn editor_agent_reference(&self) -> Option<String> {
        let target = self.editor_command_target()?;
        let lines = self.native_docs.get(&target.path)?.selected_lines();
        let root = self.selected_project().map(|project| project.path.clone());
        Some(agent_reference(
            &target.path,
            root.as_deref(),
            lines.0,
            lines.1,
        ))
    }
}

#[cfg(test)]
mod agent_reference_tests {
    use super::agent_reference;
    use std::path::Path;

    #[test]
    fn agent_reference_is_project_relative_with_line_range() {
        let root = Path::new("/work/app");
        let file = Path::new("/work/app/src/main.rs");
        assert_eq!(agent_reference(file, Some(root), 4, 4), "@src/main.rs#L4");
        assert_eq!(agent_reference(file, Some(root), 4, 9), "@src/main.rs#L4-9");
        assert_eq!(
            agent_reference(Path::new("/elsewhere/x.rs"), Some(root), 1, 1),
            "@/elsewhere/x.rs#L1"
        );
    }
}
