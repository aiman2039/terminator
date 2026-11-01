//! Workspace trust for language servers. A server runs project code
//! (`build.rs`, proc macros, TypeScript plugins), so no server spawns for
//! a project root until the user trusts it. Unknown roots queue an
//! in-app question; the app persists every answer.
use super::*;

impl LspManager {
    /// Load the saved answers (`root -> trusted`) from preferences.
    pub fn set_trust_decisions(&mut self, decisions: &HashMap<PathBuf, bool>) {
        self.trust.clone_from(decisions);
        self.trust_asks
            .retain(|root| !self.trust.contains_key(root));
    }

    /// Record the user's answer for one root. Trusting lets the next
    /// frame's sync spawn the server; refusing keeps it off for good
    /// (until the saved answer changes).
    pub fn set_trust(&mut self, root: &Path, trusted: bool) {
        self.trust.insert(root.to_path_buf(), trusted);
        self.trust_asks.retain(|asked| asked != root);
    }

    /// Project root waiting for a trust answer for this buffer, if any.
    #[must_use]
    pub fn trust_request_for(&self, path: &Path) -> Option<&Path> {
        let root = &self.docs.get(path)?.server.root;
        self.trust_asks
            .iter()
            .find(|asked| *asked == root)
            .map(PathBuf::as_path)
    }

    /// Whether servers may start in `root`. An unknown root queues one
    /// question (shown by the editor bar) and stays off meanwhile.
    pub(super) fn root_trusted(&mut self, root: &Path) -> bool {
        if self.trust_all {
            return true;
        }
        if let Some(trusted) = self.trust.get(root) {
            return *trusted;
        }
        if !self.trust_asks.iter().any(|asked| asked == root) {
            self.trust_asks.push(root.to_path_buf());
        }
        false
    }

    /// Status note for a buffer whose root is not trusted (yet).
    pub(super) fn trust_note(&self, root: &Path) -> Option<String> {
        if self.trust_all {
            return None;
        }
        match self.trust.get(root) {
            Some(true) => None,
            Some(false) => Some("Language server off: project not trusted".to_owned()),
            None => Some("Language server waits for project trust".to_owned()),
        }
    }

    /// Test managers start with trust already granted everywhere, so
    /// flows unrelated to the trust gate keep spawning servers.
    #[cfg(test)]
    pub(super) fn trusting() -> Self {
        let mut manager = Self::new();
        manager.trust_all = true;
        manager
    }
}
