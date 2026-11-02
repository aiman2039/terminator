use super::UiPreferences;

impl UiPreferences {
    /// Workspace tab nodes are expanded by default and only collapsed ones
    /// are stored. Older files kept an expanded key per tab ever shown;
    /// drop those so the file does not grow without limit.
    pub(super) fn prune_expanded_tabs(&mut self) {
        self.expanded
            .retain(|key, open| !*open || !key.starts_with("workspace-tab:"));
    }
}
