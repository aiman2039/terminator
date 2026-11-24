//! The "Needs you" and "Runs" sections of the Flows tool window. Needs
//! you holds every gate and question of the shown runs that did not end,
//! as amber cards with Approve/Reject or an answer field: an explicit click
//! answers. In Runs, only the newest run and the runs that did not end show
//! their steps; an older ended run is one line that a click opens. The
//! open state is GUI memory only.
use super::Run;
use super::panel::{Act, ROW_HEIGHT, run_status, section_title};
use super::status::{self, Tone};
use crate::*;
use std::sync::Arc;
use terminator_core::flow::{StepRun, StepStatus};

/// Most text rows of a note under a step; the full text is on hover.
const NOTE_ROWS: usize = 2;

/// First managed worktree path used by any step or racer of `run`.
fn run_worktree(run: &Run) -> Option<std::path::PathBuf> {
    run.state.order.iter().find_map(|id| {
        let step = run.state.steps.get(id)?;
        step.worktree_path
            .clone()
            .or_else(|| step.racers.iter().find_map(|r| r.worktree_path.clone()))
    })
}

/// "stopped · 1 of 4 steps done": the hover text of a run.
pub(super) fn run_summary(run: &Run) -> String {
    let (done, total) = done_of(run);
    format!("{} · {done} of {total} steps done", run_status(run))
}

/// Steps finished (done or skipped), and all steps.
fn done_of(run: &Run) -> (usize, usize) {
    let steps = run.state.steps.values();
    let done = steps.clone().filter(|step| step.status.passed()).count();
    (done, steps.count())
}

/// The trailing text of a run row: "1/2" while it runs, else when it
/// started ("2h ago", from the second in its folder name).
pub(super) fn run_trailing(run: &Run, now: u64) -> String {
    if !run.ended() {
        let (done, total) = done_of(run);
        return format!("{done}/{total}");
    }
    started_at(run).map_or_else(String::new, |at| agent_presence::ago(at, now))
}

/// The second in a run's folder name (`NAME-SECONDS` or `NAME-SECONDS-N`),
/// when it has one.
fn started_at(run: &Run) -> Option<u64> {
    let name = run.dir.file_name()?.to_str()?;
    let (prefix, last) = name.rsplit_once('-')?;
    if let Ok(second) = last.parse::<u64>()
        && second > 1_000_000_000
    {
        return Some(second);
    }
    let (_, second) = prefix.rsplit_once('-')?;
    second.parse().ok().filter(|second| *second > 1_000_000_000)
}

/// One gate or question that waits for the user.
pub(super) struct Need<'a> {
    pub(super) run: &'a Run,
    pub(super) step: &'a str,
    pub(super) state: &'a StepRun,
}

/// Every gate and question that waits in `runs`, in run order and then
/// in flow order. Ended runs (a gone runner too) wait for nothing.
pub(super) fn needs_you<'a>(runs: &[&'a Run]) -> Vec<Need<'a>> {
    runs.iter()
        .filter(|run| !run.ended())
        .flat_map(|run| {
            run.state.order.iter().filter_map(|id| {
                let state = run.state.steps.get(id)?;
                status::waits_for_user(state).then_some(Need {
                    run,
                    step: id,
                    state,
                })
            })
        })
        .collect()
}

/// A note laid out in at most [`NOTE_ROWS`] rows of `width`.
pub(super) fn note_galley(ui: &egui::Ui, text: &str, width: f32) -> Arc<egui::Galley> {
    let font = egui::TextStyle::Small.resolve(ui.style());
    let mut job =
        egui::text::LayoutJob::simple(text.to_owned(), font, ui.visuals().weak_text_color(), width);
    job.wrap.max_rows = NOTE_ROWS;
    ui.painter().layout_job(job)
}

/// A short note under a step row: clipped, with the full text on hover
/// when clipped.
fn clipped_note(ui: &mut egui::Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(26.0);
        let galley = note_galley(ui, text, ui.available_width().max(0.0));
        let (rect, response) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
        let elided = galley.elided;
        let color = ui.visuals().weak_text_color();
        ui.painter().galley(rect.min, galley, color);
        if elided {
            response.on_hover_text(text);
        }
    });
}

impl App {
    /// The newest run (`index` 0) and a run that did not end are open
    /// unless the user closed them; other runs are closed unless opened.
    pub(super) fn run_expanded(&self, index: usize, run: &Run) -> bool {
        self.flows
            .expanded
            .get(&run.dir)
            .copied()
            .unwrap_or(index == 0 || !run.ended())
    }

    /// The gates and questions that wait, at the top, as amber cards.
    pub(super) fn needs_you_section(
        &mut self,
        ui: &mut egui::Ui,
        runs: &[&Run],
        act: &mut Option<Act>,
    ) {
        let needs = needs_you(runs);
        if needs.is_empty() {
            return;
        }
        section_title(ui, "Needs you", needs.len());
        let amber = Tone::Waits.color(&self.theme);
        for need in needs {
            egui::Frame::new()
                .fill(amber.gamma_multiply(0.06))
                .stroke(egui::Stroke::new(1.0, amber.gamma_multiply(0.5)))
                .corner_radius(appearance::RADIUS_MEDIUM)
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.need_card(ui, &need, act);
                });
            ui.add_space(6.0);
        }
    }

    /// "Flow › step", the gate message or the question, and its answer.
    fn need_card(&mut self, ui: &mut egui::Ui, need: &Need<'_>, act: &mut Option<Act>) {
        let place = format!("{} › {}", need.run.state.flow, need.step);
        let place = ui.label(
            egui::RichText::new(place)
                .size(appearance::FONT_CAPTION)
                .weak(),
        );
        if let Some(session) = &need.state.session {
            let place = place.interact(egui::Sense::click());
            if place
                .on_hover_text("Show the terminal of this step")
                .clicked()
            {
                *act = Some(Act::Go(session.clone()));
            }
        }
        let (dir, step) = (need.run.dir.clone(), need.step.to_owned());
        if let Some(question) = &need.state.question {
            ui.label(question);
            let draft = self
                .flows
                .drafts
                .entry((dir.clone(), step.clone()))
                .or_default();
            appearance::footer_row(ui, |ui| {
                // Send sits after the field; Enter in the field also sends.
                let clicked = appearance::primary_button(ui, "Send").clicked();
                let field = ui.add(
                    appearance::singleline(draft)
                        .hint_text("Answer…")
                        .desired_width(ui.available_width()),
                );
                let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (clicked || entered) && !draft.trim().is_empty() {
                    *act = Some(Act::Answer(dir, step));
                }
            });
        } else if let Some(message) = &need.state.gate {
            ui.label(message);
            if let Some(target) = &need.state.pick {
                let racers = need
                    .run
                    .state
                    .steps
                    .get(target)
                    .map(|s| s.racers.as_slice())
                    .unwrap_or(&[]);
                let mut diff = None;
                if let Some(choice) = super::gates::pick_buttons(ui, racers, &mut diff) {
                    *act = Some(Act::Gate(dir, step, choice));
                }
                if let Some(path) = diff {
                    *act = Some(Act::OpenDiff(path));
                }
            } else if let Some(choice) = super::gates::gate_buttons(ui, &need.state.options) {
                *act = Some(Act::Gate(dir, step, choice));
            }
        }
    }

    /// The runs with a status chip, a progress bar and their steps.
    pub(super) fn runs_section(&mut self, ui: &mut egui::Ui, runs: &[&Run], act: &mut Option<Act>) {
        section_title(ui, "Runs", runs.len());
        if runs.is_empty() {
            ui.weak("No runs yet. Run a flow to start one.");
            return;
        }
        let now = terminator_core::now();
        for (index, run) in runs.iter().enumerate() {
            let open = self.run_expanded(index, run);
            self.flow_run_header(ui, (run, open, now), act);
            if !run.state.inputs.is_empty() {
                let line = super::inputs_form::inputs_line(&run.state.inputs);
                clipped_note(ui, &line);
            }
            status::progress_bar(ui, run, &self.theme);
            if let Some(wt) = run_worktree(run) {
                self.flow_checks_ui(ui, &wt);
            }
            if open {
                for id in &run.state.order {
                    if let Some(step) = run.state.steps.get(id) {
                        self.flow_step_row(ui, (run, id, step), act);
                    }
                }
                self.flow_review_checklist(ui, run, act);
            }
            ui.add_space(4.0);
        }
    }

    /// One line: open/close, flow name, status chip, and Remove for an
    /// ended run or Cancel for a running one. The chip and the action come
    /// first, so a long name never hides them.
    fn flow_run_header(
        &mut self,
        ui: &mut egui::Ui,
        (run, open, now): (&Run, bool, u64),
        act: &mut Option<Act>,
    ) {
        let name = terminator_core::flow::control::run_name(&run.dir);
        let (word, tone) = status::run_chip(run);
        let trailing = run_trailing(run, now);
        let tint = appearance::color(&self.theme.text);
        let gone = if run.gone {
            " Its runner is gone (its terminal closed or the process stopped), \
             so nothing changes this run again."
        } else {
            ""
        };
        let what = if open { "hide" } else { "show" };
        let hover = format!(
            "Run {name}: {}.{gone} Click to {what} its steps.",
            run_summary(run)
        );
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if run.ended() {
                    let tip = "Remove this run from the list. Its terminals stay open.";
                    if appearance::sidebar_action(ui, "Trash", tip).clicked() {
                        *act = Some(Act::Remove(run.dir.clone()));
                    }
                    let report_tip = "Export Markdown summary of this ended run";
                    if appearance::sidebar_action(ui, "FileText", report_tip).clicked() {
                        *act = Some(Act::ExportReport(run.dir.clone()));
                    }
                } else if appearance::sidebar_action(ui, "Square", "Cancel run…").clicked() {
                    *act = Some(Act::AskCancel(run.dir.clone()));
                }
                if let Some(wt) = run_worktree(run)
                    && appearance::sidebar_action(ui, "ShieldCheck", "Load CI/PR checks & TODOs")
                        .clicked()
                {
                    *act = Some(Act::LoadChecks(wt));
                }
                status::chip(ui, word, tone.color(&self.theme)).on_hover_text(run_summary(run));
                let row = appearance::row(appearance::RowInput {
                    ui,
                    label: &run.state.flow,
                    icon: if open { "ChevronDown" } else { "ChevronRight" },
                    selected: false,
                    height: ROW_HEIGHT,
                    trailing: &trailing,
                    tint,
                })
                .on_hover_text(hover);
                if row.clicked() {
                    *act = Some(Act::Toggle(run.dir.clone(), !open));
                }
            });
        });
    }

    /// One step: its colored state icon (the loader turns), its ID, what it
    /// waits for and its attempt, and Retry for a failed step of a live
    /// run. A click shows its terminal. Under it, the stall badge.
    fn flow_step_row(
        &mut self,
        ui: &mut egui::Ui,
        (run, id, step): (&Run, &String, &StepRun),
        act: &mut Option<Act>,
    ) {
        let style = status::step_style(step);
        // A step that waits shows what it waits for; the others their state.
        let mut state = match step.status {
            StepStatus::Waiting if !step.needs.is_empty() => {
                format!("after {}", step.needs.join(", "))
            }
            _ => style.label.to_lowercase(),
        };
        if let Some(attempt) = status::attempt_label(step) {
            state.push_str(&format!(" · {attempt}"));
        }
        let minutes = self.preferences.flow_stall_minutes;
        let stall = status::stall(step, &self.state, terminator_core::now(), minutes);
        if status::can_retry(run, step) || step.session.is_some() {
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if status::can_retry(run, step) {
                        let tip = "Retry: start this step again in a new terminal";
                        if appearance::sidebar_action(ui, "RotateCw", tip).clicked() {
                            *act = Some(Act::Retry(run.dir.clone(), id.clone()));
                        }
                    }
                    if let Some(session) = &step.session {
                        let tip = "Peek screen (read-only)";
                        if appearance::sidebar_action(ui, "Eye", tip).clicked() {
                            *act = Some(Act::Peek(session.clone()));
                        }
                    }
                    self.flow_step_line(ui, (id, step, &state), act);
                });
            });
        } else {
            self.flow_step_line(ui, (id, step, &state), act);
        }
        // The choice, the condition and the round, under the row: the
        // row's trailing text stays short.
        let labels = super::gates::branch_labels(step);
        if !labels.is_empty() {
            clipped_note(ui, &labels.join(" · "));
        }
        if let Some(stall) = stall {
            clipped_note(ui, &stall);
        }
        // What a running step waits for, and the note of a failed one.
        if matches!(step.status, StepStatus::Running | StepStatus::Failed) {
            for text in [&step.waits_for, &step.note].into_iter().flatten() {
                clipped_note(ui, text);
            }
        }
        if let Some(by) = &step.proposed_by {
            clipped_note(ui, &format!("Proposed by {by}"));
        }
        if let Some(port) = step.port {
            clipped_note(ui, &format!("Port: {port}"));
        }
        if !step.outputs.is_empty() {
            clipped_note(ui, &format!("Outputs: {}", step.outputs.join(", ")));
        }
        Self::flow_step_racers(ui, step, act);
        self.flow_step_mailbox(ui, (run, id, step), act);
    }

    fn flow_step_racers(ui: &mut egui::Ui, step: &StepRun, act: &mut Option<Act>) {
        for racer in &step.racers {
            let label = match &racer.agent {
                Some(agent) => format!("#{} ({agent})", racer.index),
                None => format!("#{}", racer.index),
            };
            let diffstat = match (racer.added, racer.deleted) {
                (Some(a), Some(d)) => format!(" +{a} −{d}"),
                _ => String::new(),
            };
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.weak(format!("{label}: {:?}{diffstat}", racer.status));
                if let Some(session) = &racer.session
                    && appearance::link(
                        ui,
                        egui::RichText::new("Terminal").size(appearance::FONT_CAPTION),
                    )
                    .clicked()
                {
                    *act = Some(Act::Go(session.clone()));
                }
                if let Some(path) = &racer.worktree_path
                    && appearance::link(
                        ui,
                        egui::RichText::new("Diff").size(appearance::FONT_CAPTION),
                    )
                    .clicked()
                {
                    *act = Some(Act::OpenDiff(path.clone()));
                }
            });
        }
    }

    fn flow_step_mailbox(
        &mut self,
        ui: &mut egui::Ui,
        (run, id, step): (&Run, &String, &StepRun),
        act: &mut Option<Act>,
    ) {
        if run.ended()
            || step.status != StepStatus::Running
            || step.gate.is_some()
            || step.question.is_some()
        {
            return;
        }
        if step.inbox > 0 {
            clipped_note(
                ui,
                &format!(
                    "Mailbox: {}",
                    appearance::count_noun(step.inbox as usize, "note")
                ),
            );
        }
        let key = (run.dir.clone(), id.clone());
        let draft = self.flows.mail_drafts.entry(key).or_default();
        let mut send = false;
        ui.horizontal(|ui| {
            ui.add_space(20.0);
            let filled = !draft.trim().is_empty();
            let width = (ui.available_width() - 56.0).max(60.0);
            let field = ui.add(
                appearance::singleline(draft)
                    .hint_text("Send note to inbox…")
                    .desired_width(width),
            );
            let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let clicked = ui
                .add_enabled(filled, egui::Button::new("Send"))
                .on_hover_text("Store a note in this step's mailbox")
                .clicked();
            send = (clicked || entered) && filled;
        });
        if send {
            *act = Some(Act::SendMail(run.dir.clone(), id.clone()));
        }
    }

    /// The row of one step: icon, ID and `state`. A click shows its
    /// terminal.
    fn flow_step_line(
        &self,
        ui: &mut egui::Ui,
        (id, step, state): (&String, &StepRun, &str),
        act: &mut Option<Act>,
    ) {
        let style = status::step_style(step);
        let hover = if step.session.is_some() {
            format!("{}. Click to show the terminal of this step.", style.label)
        } else {
            style.label.to_owned()
        };
        let row = appearance::status_row(appearance::StatusRowInput {
            row: appearance::RowInput {
                ui,
                label: id,
                icon: style.icon,
                selected: false,
                height: ROW_HEIGHT,
                trailing: state,
                tint: appearance::color(&self.theme.text),
            },
            icon_tint: style.color(&self.theme),
            spin: style.spin,
        })
        .on_hover_text(hover);
        if row.clicked()
            && let Some(session) = &step.session
        {
            *act = Some(Act::Go(session.clone()));
        }
    }
}
