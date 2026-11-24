//! Gates, branches and loops in the runner: a gate's announcement and
//! answer (with an option), the steps to skip, and a loop's next round.
//! Nothing here starts a terminal; the runner starts the steps that wait.
use super::{
    ANSWERS,
    retry::clear_answers,
    runner::{End, Runner},
};
use crate::output::Output;
use terminator_core::flow::{Step, StepRun, control::GateAnswer};

impl Runner {
    /// Prints the gate's answer commands and raises a notice on the
    /// terminal of a step it needs.
    pub(super) fn announce_gate(
        &self,
        step: &Step,
        (message, options, pick): (&str, &[String], Option<&str>),
    ) {
        let run = terminator_core::flow::control::run_name(&self.dir);
        let id = &step.id;
        let approve = if let Some(target) = pick {
            let count = self.state.steps.get(target).map_or(0, |s| s.racers.len());
            (1..=count)
                .map(|i| format!("  terminator-hook flow approve {run} {id} --pick {i}"))
                .collect::<Vec<_>>()
                .join("\n")
        } else if options.is_empty() {
            format!("  terminator-hook flow approve {run} {id}")
        } else {
            let lines = options.iter().map(|option| {
                format!("  terminator-hook flow approve {run} {id} --option {option}")
            });
            lines.collect::<Vec<_>>().join("\n")
        };
        Output::line_or_drop(format_args!(
            "{id}: gate: {message}\n{approve}\n  terminator-hook flow reject {run} {id}"
        ));
        let session = step
            .needs
            .iter()
            .find_map(|needs| self.state.steps.get(needs)?.session.clone());
        if let Some(session) = session {
            self.notify(session, "Flow gate", message);
        }
    }

    /// The answer that `flow approve` or `flow reject` wrote. A gate with
    /// options takes only one of its options; a pick gate takes `--pick N`;
    /// one without takes only Approve.
    pub(super) fn gate_answer(&mut self, id: &str) -> End {
        let text = std::fs::read_to_string(self.dir.join(ANSWERS).join(id)).ok()?;
        let pick_count = self
            .state
            .steps
            .get(id)
            .and_then(|run| run.pick.as_deref())
            .and_then(|target| self.state.steps.get(target))
            .map(|race| race.racers.len());
        let run = self.state.steps.get_mut(id)?;
        match GateAnswer::parse(&text)? {
            GateAnswer::Pick(index)
                if pick_count.is_some_and(|count| index >= 1 && (index as usize) <= count) =>
            {
                run.picked = Some(index);
                run.choice = Some(format!("#{index}"));
                Some(Ok(()))
            }
            GateAnswer::Approve if run.options.is_empty() && run.pick.is_none() => Some(Ok(())),
            GateAnswer::Choose(option) if run.options.iter().any(|name| name == option) => {
                run.choice = Some(option.to_owned());
                Some(Ok(()))
            }
            GateAnswer::Reject => Some(Err("Rejected".to_owned())),
            GateAnswer::Approve | GateAnswer::Choose(_) | GateAnswer::Pick(_) => None,
        }
    }

    /// Marks the steps whose `when` option was not chosen, or whose needed
    /// steps were all skipped, as skipped.
    pub(super) fn skips(&mut self) {
        for id in self.flow.skip(&mut self.state) {
            self.changed = true;
            Output::line_or_drop(format_args!("{id}: skipped"));
        }
    }

    /// A loop step `id` is done: before its last round, the loop's steps
    /// wait again as the next round. Their old reports, answers and
    /// questions go; their terminals stay.
    pub(super) fn loop_back(&mut self, id: &str) {
        let again = self.flow.loop_back(&mut self.state, id);
        if again.is_empty() {
            return;
        }
        for step in &again {
            clear_answers(&self.dir, step);
            self.phases.remove(step);
        }
        let label = self.state.steps.get(id).and_then(StepRun::round_label);
        Output::line_or_drop(format_args!(
            "{id}: loop, {}: {}",
            label.unwrap_or_default(),
            again.join(", ")
        ));
    }

    /// For the first step of a loop in a later round: the loop step and
    /// its note of the round before, which the prompt gets.
    pub(super) fn loop_note(&self, id: &str) -> Option<(&str, &str)> {
        let note = self.state.steps.get(id)?.loop_note.as_deref()?;
        let from = self
            .flow
            .steps
            .iter()
            .find(|step| step.loop_back.as_ref().is_some_and(|back| back.to == id))?;
        Some((from.id.as_str(), note))
    }
}
