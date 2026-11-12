//! Ask agent: type a short prompt and paste it into a live agent terminal.
//! Enter pastes without Enter (the user sends it); ⌘Enter also sends it.
//! An explicit user action, never automatic.
use super::*;

/// The open Ask agent prompt.
#[derive(Clone, Debug, Default)]
pub(crate) struct AskAgent {
    pub(crate) prompt: String,
    /// `@path#L…` of the focused editor's selection when the prompt opened.
    pub(crate) reference: Option<String>,
    pub(crate) include_reference: bool,
    /// Session that receives the prompt.
    pub(crate) target: Option<String>,
    focus: bool,
    /// Repository whose diff review notes go with the prompt.
    pub(crate) notes_repo: Option<PathBuf>,
    /// Those notes on one line, refreshed while the dialog is open.
    pub(crate) notes: String,
    /// A note the user removed in the dialog this frame.
    remove_note: Option<usize>,
}

/// The text pasted into the agent: the reference first, then the prompt,
/// then the review notes. It is one line: pasting it never presses Enter.
pub(crate) fn ask_text(ask: &AskAgent) -> String {
    let reference = ask.reference.as_deref().filter(|_| ask.include_reference);
    let parts: Vec<&str> = [reference.unwrap_or(""), ask.prompt.trim(), ask.notes.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect();
    let text = crate::diff::notes::one_line(&parts.join(" "));
    // A lone reference keeps a space so the user types right after it.
    if reference.is_some() && parts.len() == 1 {
        format!("{text} ")
    } else {
        text
    }
}

impl App {
    /// Live agent terminals the GUI can type into, the active one first,
    /// then the newest.
    pub(crate) fn ask_targets(&self) -> Vec<(String, String)> {
        let mut sessions: Vec<&Session> = self
            .state
            .sessions
            .iter()
            .filter(|session| {
                session.lifecycle.live()
                    && self.backends.contains_key(&session.id)
                    && self.present_session(&session.id).live
            })
            .collect();
        sessions.sort_by_key(|session| {
            (
                self.active_session.as_ref() != Some(&session.id),
                std::cmp::Reverse(session.created),
            )
        });
        sessions
            .into_iter()
            .map(|session| (session.id.clone(), session.label.clone()))
            .collect()
    }

    pub(crate) fn open_ask_agent(&mut self) {
        let targets = self.ask_targets();
        if targets.is_empty() {
            self.info = Some("No open agent terminal to ask.".into());
            return;
        }
        let reference = self.editor_agent_reference();
        self.palette.ask = Some(AskAgent {
            include_reference: reference.is_some(),
            reference,
            target: targets.first().map(|(id, _)| id.clone()),
            focus: true,
            ..AskAgent::default()
        });
    }

    /// Ask agent with the diff review notes of `repo`: the user picks the
    /// agent and chooses Paste or Paste and send.
    pub(crate) fn open_ask_with_notes(&mut self, repo: PathBuf) {
        let targets = self.ask_targets();
        if targets.is_empty() {
            self.info = Some("No open agent terminal to send the notes to.".into());
            return;
        }
        self.palette.ask = Some(AskAgent {
            target: targets.first().map(|(id, _)| id.clone()),
            notes_repo: Some(repo),
            focus: true,
            ..AskAgent::default()
        });
    }

    /// Paste the prompt into the chosen agent; `enter` also sends it.
    /// True when something was pasted.
    pub(crate) fn send_ask(&mut self, ask: &AskAgent, enter: bool) -> bool {
        let Some(session) = ask.target.as_deref() else {
            return false;
        };
        if !self.paste_prompt(session, &ask_text(ask), enter) {
            return false;
        }
        // Sent notes are done; they do not go again with the next prompt.
        if let Some(repo) = &ask.notes_repo {
            self.diff_notes.clear(repo);
        }
        true
    }

    /// Pastes `text` into the live agent terminal `session`; `enter` also
    /// presses Enter. The one path for Ask agent and the prompt composer;
    /// only an explicit click or key calls it. True when pasted.
    pub(crate) fn paste_prompt(&mut self, session: &str, text: &str, enter: bool) -> bool {
        // The agent can exit while the dialog is open: never type into
        // what is now a plain shell.
        if !self.ask_targets().iter().any(|(id, _)| id == session) {
            self.info = Some("The agent is no longer running. Nothing was sent.".into());
            return false;
        }
        if text.is_empty() {
            return false;
        }
        self.handle_clipboard_paste(session, text);
        if enter && let Some(backend) = self.backends.get_mut(session) {
            backend.process_command(egui_term::BackendCommand::Write(b"\r".to_vec()));
        }
        self.go_session(session);
        true
    }

    /// The review notes as one line, with references relative to the
    /// target agent's project.
    fn refresh_ask_notes(&self, ask: &mut AskAgent) {
        let Some(repo) = &ask.notes_repo else {
            return;
        };
        let root = ask.target.as_deref().and_then(|target| {
            let session = self.state.sessions.iter().find(|s| s.id == target)?;
            let project = self
                .state
                .projects
                .iter()
                .find(|p| p.id == session.project_id)?;
            Some(project.path.clone())
        });
        ask.notes = crate::diff::notes::batch_text(self.diff_notes.list(repo), root.as_deref());
    }

    pub(crate) fn ask_agent_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut ask) = self.palette.ask.take() else {
            return;
        };
        self.refresh_ask_notes(&mut ask);
        let targets = self.ask_targets();
        let mut open = true;
        let mut send = None;
        self.popups
            .window(ctx, "Ask agent")
            .open(&mut open)
            .collapsible(false)
            .show(ctx, |ui| {
                send = self.ask_agent_form(ui, &mut ask, &targets);
            });
        if let (Some(repo), Some(index)) = (&ask.notes_repo, ask.remove_note.take()) {
            self.diff_notes.remove(repo, index);
        }
        if let Some(enter) = send {
            self.send_ask(&ask, enter);
        } else if open && !ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.palette.ask = Some(ask);
        }
    }

    /// The review notes the prompt carries, each with a remove button.
    fn ask_notes_list(&self, ui: &mut egui::Ui, ask: &mut AskAgent) {
        let Some(repo) = &ask.notes_repo else {
            return;
        };
        let notes = self.diff_notes.list(repo);
        ui.weak(format!("{} review note(s)", notes.len()));
        for (index, note) in notes.iter().enumerate() {
            ui.horizontal(|ui| {
                if appearance::sidebar_action(ui, "X", "Remove this note").clicked() {
                    ask.remove_note = Some(index);
                }
                let name = note.path.file_name().unwrap_or_default().to_string_lossy();
                let lines = if note.first == note.last {
                    format!("{name}:{}", note.first)
                } else {
                    format!("{name}:{}-{}", note.first, note.last)
                };
                ui.add(egui::Label::new(format!("{lines}  {}", note.text)).truncate());
            });
        }
    }

    /// Returns `Some(enter)` when the prompt is sent.
    fn ask_agent_form(
        &self,
        ui: &mut egui::Ui,
        ask: &mut AskAgent,
        targets: &[(String, String)],
    ) -> Option<bool> {
        let field = ui.add(
            appearance::singleline(&mut ask.prompt)
                .hint_text("Ask the agent…")
                .desired_width(420.0),
        );
        if std::mem::take(&mut ask.focus) {
            field.request_focus();
        }
        let enter = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        let command = ui.input(|input| input.modifiers.command);
        if let Some(reference) = &ask.reference {
            ui.checkbox(&mut ask.include_reference, reference);
        }
        self.ask_notes_list(ui, ask);
        ui.weak("Agent");
        for (id, label) in targets {
            let presented = self.present_session(id);
            let state = presented.row_state(now()).unwrap_or_default();
            let text =
                RichText::new(format!("{label}  {state}")).color(presented.lifecycle.map_or_else(
                    || ui.visuals().text_color(),
                    |state| crate::sidebar_ui::state_color(state, &self.theme),
                ));
            if ui
                .selectable_label(ask.target.as_ref() == Some(id), text)
                .clicked()
            {
                ask.target = Some(id.clone());
            }
        }
        let mut send = enter.then_some(command);
        appearance::footer_row(ui, |ui| {
            if ui.button("Paste and send").clicked() {
                send = Some(true);
            }
            if ui.button("Paste").clicked() {
                send = Some(false);
            }
        });
        send
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ask_text_puts_the_reference_before_the_prompt() {
        let mut ask = AskAgent {
            prompt: " explain this ".into(),
            reference: Some("@src/main.rs#L4-9".into()),
            include_reference: true,
            ..AskAgent::default()
        };
        assert_eq!(ask_text(&ask), "@src/main.rs#L4-9 explain this");
        ask.include_reference = false;
        assert_eq!(ask_text(&ask), "explain this");
        ask.prompt.clear();
        assert_eq!(ask_text(&ask), "");
    }

    #[test]
    fn review_notes_follow_the_prompt_on_one_line_without_enter() {
        let ask = AskAgent {
            prompt: "please fix\nthese".into(),
            notes: "Review notes: 1) @a.rs#L4: rename".into(),
            ..AskAgent::default()
        };
        let text = ask_text(&ask);
        assert_eq!(text, "please fix these Review notes: 1) @a.rs#L4: rename");
        // Paste (not Paste and send) writes these bytes and nothing else.
        for mode in [
            egui_term::TerminalMode::empty(),
            egui_term::TerminalMode::BRACKETED_PASTE,
        ] {
            let bytes = egui_term::paste_input(&text, mode);
            assert!(!bytes.contains(&b'\r') && !bytes.contains(&b'\n'));
        }
    }
}
