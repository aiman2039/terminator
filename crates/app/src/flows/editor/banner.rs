//! The run strip of the flow editor: one row below the toolbar with the
//! state of the newest run of the flow, one progress segment per step,
//! what the run does now, and the one action that applies: Approve and
//! Reject for a gate that waits, the answer of a question, or Cancel run…
//! (after a confirmation). A Run click has a clear result here.
use super::style::Colors;
use crate::flows::Run;
use crate::flows::status::{self, chip, step_style};
use crate::*;
use std::path::{Path, PathBuf};
use terminator_core::flow::{StepRun, StepStatus};

/// Widest run inputs line in the strip.
const INPUTS_WIDTH: f32 = 180.0;

/// One click in the run strip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum StripClick {
    /// Hide the strip of an ended run.
    Hide,
    /// Answer the gate of this step.
    Gate(String, crate::flows::gates::GateChoice),
    /// Send the typed answer of this step's question.
    Answer(String),
    /// Start this failed step again.
    Retry(String),
    /// Ask to cancel the run.
    Cancel,
}

/// The run's state as a chip: the shared run word (`status::run_chip`)
/// with a capital, and its tone's color.
fn status_chip(run: &Run, colors: Colors) -> (String, egui::Color32) {
    let (word, tone) = status::run_chip(run);
    let color = colors.tone(tone);
    let mut chars = word.chars();
    let word = chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    });
    (word, color)
}

/// The steps of `run` in their order.
fn steps(run: &Run) -> impl Iterator<Item = (&String, &StepRun)> {
    run.state
        .order
        .iter()
        .filter_map(|id| Some((id, run.state.steps.get(id)?)))
}

/// The step that waits for the user: a gate or a question.
fn waits_for_user(run: &Run) -> Option<(&String, &StepRun)> {
    if run.ended() {
        return None;
    }
    steps(run).find(|(_, step)| status::waits_for_user(step))
}

/// What the run does now: "build: waits for the agent's first hook
/// report", or the step that stopped it.
pub(super) fn now_text(run: &Run) -> Option<String> {
    if run.gone {
        return Some("Its runner is gone: nothing changes this run again.".into());
    }
    if let Some((id, step)) = waits_for_user(run) {
        let text = step
            .question
            .as_deref()
            .or(step.gate.as_deref())
            .unwrap_or("");
        return Some(format!("{id}: {text}"));
    }
    if let Some((id, step)) = steps(run).find(|(_, step)| step.status == StepStatus::Running) {
        let state = step.waits_for.as_deref().unwrap_or("runs");
        return Some(format!("{id}: {}", lower_first(state)));
    }
    let (id, step) = steps(run).find(|(_, step)| step.status == StepStatus::Failed)?;
    let note = step.note.as_deref().unwrap_or("No reason given.");
    if run.state.held() && !run.ended() {
        return Some(format!("{id} failed: {note} Retry it, or cancel the run."));
    }
    Some(format!("{id} failed: {note}"))
}

fn lower_first(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_lowercase().chain(chars).collect()
    })
}

/// One segment per step, in the color of its state.
fn progress(ui: &mut egui::Ui, run: &Run, colors: Colors) {
    let count = run.state.order.len().max(1);
    let segment = egui::vec2((96.0 / f32_from(count)).clamp(6.0, 18.0), 6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (id, step) in steps(run) {
            let style = step_style(step);
            let (rect, response) = ui.allocate_exact_size(segment, egui::Sense::hover());
            let fill = colors.wire(style);
            ui.painter()
                .rect_filled(rect, appearance::RADIUS_XSMALL, fill);
            response.on_hover_text(format!("{id}: {}", style.label));
        }
    });
}

#[expect(
    clippy::cast_precision_loss,
    reason = "A flow has at most a few dozen steps."
)]
fn f32_from(count: usize) -> f32 {
    count as f32
}

/// The frame of the strip.
fn strip_frame(colors: Colors, line: egui::Color32) -> egui::Frame {
    egui::Frame::new()
        .fill(colors.node)
        .stroke(egui::Stroke::new(1.0, line))
        .corner_radius(appearance::RADIUS_MEDIUM)
        .inner_margin(egui::Margin::symmetric(10, 4))
        .outer_margin(egui::Margin::symmetric(8, 2))
}

/// The strip of `run`. `draft` is the typed answer of the question that
/// waits, if any.
pub(super) fn run_strip(
    ui: &mut egui::Ui,
    run: &Run,
    colors: Colors,
    drafts: &mut std::collections::HashMap<(PathBuf, String), String>,
) -> Option<StripClick> {
    let (state, color) = status_chip(run, colors);
    let total = run.state.steps.len();
    let done = run
        .state
        .steps
        .values()
        .filter(|step| step.status == StepStatus::Done)
        .count();
    let mut click = None;
    strip_frame(colors, colors.border).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            chip(ui, &state, color);
            progress(ui, run, colors);
            ui.weak(format!("{done} of {total} done"));
            if !run.state.inputs.is_empty() {
                let line = crate::flows::inputs_form::inputs_line(&run.state.inputs);
                let text = egui::RichText::new(&line)
                    .size(appearance::FONT_CAPTION)
                    .weak();
                // At most a fixed width: the action at the right keeps its place.
                let size = egui::vec2(INPUTS_WIDTH, ui.spacing().interact_size.y);
                ui.allocate_ui(size, |ui| {
                    ui.add(egui::Label::new(text).truncate())
                        .on_hover_text(format!("Inputs: {line}"));
                });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                click = strip_action(ui, run, drafts);
                let now = now_text(run).unwrap_or_default();
                ui.allocate_ui_with_layout(
                    ui.available_size(),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        let label = egui::Label::new(egui::RichText::new(&now).weak()).truncate();
                        ui.add(label).on_hover_text(&now);
                    },
                );
            });
        });
    });
    ui.add_space(2.0);
    click
}

/// The one action of the strip, right to left.
fn strip_action(
    ui: &mut egui::Ui,
    run: &Run,
    drafts: &mut std::collections::HashMap<(PathBuf, String), String>,
) -> Option<StripClick> {
    if run.ended() {
        let hide = appearance::sidebar_action(ui, "X", "Hide this run strip").clicked();
        return hide.then_some(StripClick::Hide);
    }
    if let Some((id, step)) = waits_for_user(run) {
        if step.question.is_none() {
            let choice = crate::flows::gates::gate_buttons_right_to_left(ui, &step.options);
            return choice.map(|choice| StripClick::Gate(id.clone(), choice));
        }
        let draft = drafts.entry((run.dir.clone(), id.clone())).or_default();
        let filled = !draft.trim().is_empty();
        let send = ui.add_enabled(filled, egui::Button::new("Send")).clicked();
        let field = ui.add(
            appearance::singleline(draft)
                .hint_text("Answer…")
                .desired_width(200.0),
        );
        let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        return ((send || entered) && filled).then(|| StripClick::Answer(id.clone()));
    }
    let cancel = ui
        .button("Cancel run…")
        .on_hover_text("No new step starts. Running terminals continue.");
    if cancel.clicked() {
        return Some(StripClick::Cancel);
    }
    // One Retry per failed step, each with its step: right to left, so the
    // first failed step is at the left.
    let failed: Vec<&String> = steps(run)
        .filter(|(_, step)| status::can_retry(run, step))
        .map(|(id, _)| id)
        .collect();
    let mut click = None;
    for id in failed.into_iter().rev() {
        let retry = appearance::primary_button(ui, &format!("Retry {id}"))
            .on_hover_text(format!("Start {id} again in a new terminal"));
        if retry.clicked() {
            click = Some(StripClick::Retry(id.clone()));
        }
    }
    click
}

/// A Run click whose new run does not show yet.
pub(crate) struct Starting {
    path: PathBuf,
    /// The clock at the click, in seconds: a run folder's name ends with
    /// the second in which its runner started.
    at: u64,
    since: Instant,
}

/// The longest time that the strip says "Starting".
const STARTING_LIMIT: Duration = Duration::from_secs(30);

impl Starting {
    pub(super) fn now(path: &Path) -> Self {
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |time| time.as_secs());
        Self {
            path: path.into(),
            at,
            since: Instant::now(),
        }
    }

    /// True for a run that started at or after the click.
    fn is_new(&self, run: &Run) -> bool {
        run.dir
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.rsplit('-').next())
            .and_then(|second| second.parse::<u64>().ok())
            .is_some_and(|second| second.saturating_add(1) >= self.at)
    }
}

/// True while the editor of `path` waits for the run of its last Run
/// click: `run` is then an older run. Forgets the click when its run
/// shows or after a time limit.
pub(super) fn still_starting(
    starting: &mut Option<Starting>,
    path: &Path,
    run: Option<&Run>,
) -> bool {
    let Some(click) = starting.as_ref().filter(|click| click.path == path) else {
        return false;
    };
    let shows = run.is_some_and(|run| click.is_new(run));
    if shows || click.since.elapsed() > STARTING_LIMIT {
        *starting = None;
        return false;
    }
    true
}

/// The strip between a Run click and the first state of its run.
pub(super) fn starting_strip(ui: &mut egui::Ui, colors: Colors) {
    strip_frame(colors, colors.border).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(egui::RichText::new("Starting the run…").color(colors.step));
            ui.weak("The runner starts in its own terminal tab.");
        });
    });
    ui.add_space(2.0);
}
