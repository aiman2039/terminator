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
}

/// The text pasted into the agent: the reference first, then the prompt.
pub(crate) fn ask_text(ask: &AskAgent) -> String {
    match ask.reference.as_deref().filter(|_| ask.include_reference) {
        Some(reference) if ask.prompt.trim().is_empty() => format!("{reference} "),
        Some(reference) => format!("{reference} {}", ask.prompt.trim()),
        None => ask.prompt.trim().to_owned(),
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

    /// Paste the prompt into the chosen agent; `enter` also sends it.
    pub(crate) fn send_ask(&mut self, ask: &AskAgent, enter: bool) {
        let Some(session) = ask.target.as_deref() else {
            return;
        };
        // The agent can exit while the dialog is open: never type into
        // what is now a plain shell.
        if !self.ask_targets().iter().any(|(id, _)| id == session) {
            self.info = Some("The agent is no longer running. Nothing was sent.".into());
            return;
        }
        let text = ask_text(ask);
        if text.is_empty() {
            return;
        }
        self.handle_clipboard_paste(session, &text);
        if enter && let Some(backend) = self.backends.get_mut(session) {
            backend.process_command(egui_term::BackendCommand::Write(b"\r".to_vec()));
        }
        self.go_session(session);
    }

    pub(crate) fn ask_agent_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut ask) = self.palette.ask.take() else {
            return;
        };
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
        if let Some(enter) = send {
            self.send_ask(&ask, enter);
        } else if open && !ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.palette.ask = Some(ask);
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
}
