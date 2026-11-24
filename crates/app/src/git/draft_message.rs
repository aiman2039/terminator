//! Draft commit message: opens Ask agent with a fixed prompt for a live
//! agent terminal. Paste types it without Enter; only Paste and send sends
//! it. No built-in model, and nothing commits without the Commit click.
use crate::*;

/// The prompt pasted into the agent.
pub(crate) const DRAFT_PROMPT: &str = "Write a commit message for the staged changes (git diff --staged), in the style of the last commits (git log -5). Reply with the message only; do not commit.";
pub(crate) const PR_FIX_CI_PROMPT: &str = "Inspect the failing CI checks on this branch's pull request, fix the root causes, run local tests, and summarize what you changed.";
pub(crate) const PR_FIX_COMMENTS_PROMPT: &str = "Review the open review comments on this branch's pull request, apply the requested fixes, run local tests, and summarize each change.";
pub(crate) const PR_RESOLVE_CONFLICTS_PROMPT: &str = "Inspect the merge conflicts between this branch and its base branch, resolve them cleanly without losing intent, run local tests, and summarize.";

impl App {
    pub(crate) fn ask_agent_with_prompt(&mut self, prompt: &str) {
        self.open_ask_agent();
        if let Some(ask) = self.palette.ask.as_mut() {
            prompt.clone_into(&mut ask.prompt);
            ask.include_reference = false;
        }
    }

    pub(crate) fn draft_commit_message(&mut self) {
        self.ask_agent_with_prompt(DRAFT_PROMPT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_without_a_live_agent_says_so_and_opens_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, terminator_core::Paths::at(dir.path().into()));
        app.draft_commit_message();
        assert!(app.palette.ask.is_none());
        assert_eq!(app.info.as_deref(), Some("No open agent terminal to ask."));
    }

    #[test]
    fn the_draft_prompt_has_no_line_break_that_could_send_it() {
        for prompt in [
            DRAFT_PROMPT,
            PR_FIX_CI_PROMPT,
            PR_FIX_COMMENTS_PROMPT,
            PR_RESOLVE_CONFLICTS_PROMPT,
        ] {
            assert!(!prompt.contains(['\n', '\r']), "{prompt:?}");
        }
    }
}
