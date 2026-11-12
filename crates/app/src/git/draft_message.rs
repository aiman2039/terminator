//! Draft commit message: opens Ask agent with a fixed prompt for a live
//! agent terminal. Paste types it without Enter; only Paste and send sends
//! it. No built-in model, and nothing commits without the Commit click.
use crate::*;

/// The prompt pasted into the agent.
pub(crate) const DRAFT_PROMPT: &str = "Write a commit message for the staged changes (git diff --staged), in the style of the last commits (git log -5). Reply with the message only; do not commit.";

impl App {
    pub(crate) fn draft_commit_message(&mut self) {
        self.open_ask_agent();
        if let Some(ask) = self.palette.ask.as_mut() {
            DRAFT_PROMPT.clone_into(&mut ask.prompt);
            ask.include_reference = false;
        }
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
        assert!(!DRAFT_PROMPT.contains(['\n', '\r']));
    }
}
