//! OSC 7501 program status (`program-status-v1`), display only: a pane
//! badge and, for `blocked` and `done`, a terminal notice like OSC 9. It
//! never feeds agent lifecycle, presence or agent notifications.
use super::Events;
use terminator_core::program_status::{self, ProgramStatus, Report};

/// The latest root status of one session and its publication throttle.
#[derive(Default)]
pub struct ProgramSlot {
    current: Option<ProgramStatus>,
    dirty: bool,
    published: Option<std::time::Instant>,
}

impl Events {
    pub(super) fn program_status(&mut self, params: &[&[u8]]) {
        match program_status::parse(params) {
            Some(Report::Query) => self.reply(program_status::QUERY_REPLY.into()),
            Some(Report::Clear) => self.set_program_status(None),
            Some(Report::Set(status)) => {
                let changed = self
                    .program
                    .current
                    .as_ref()
                    .is_none_or(|old| old.state != status.state);
                if changed && let Some((title, body)) = program_status::notice(&status) {
                    self.notice(title, body);
                }
                self.set_program_status(Some(status));
            }
            None => {}
        }
    }

    /// A new shell prompt ends a `working` or `blocked` status.
    pub(super) fn prompt_started(&mut self) {
        if self
            .program
            .current
            .as_ref()
            .is_some_and(|status| status.state.transient())
        {
            self.set_program_status(None);
        }
    }

    fn set_program_status(&mut self, status: Option<ProgramStatus>) {
        if self.program.current != status {
            self.program.current = status;
            self.program.dirty = true;
        }
    }

    /// The pending status change, when one is due (same throttle as the
    /// window title).
    pub fn take_program_status(
        &mut self,
        now: std::time::Instant,
        force: bool,
    ) -> Option<StatusChange> {
        let slot = &mut self.program;
        let due = force
            || slot.published.is_none_or(|at| {
                now.saturating_duration_since(at) >= super::TITLE_PUBLISH_INTERVAL
            });
        if !slot.dirty || !due {
            return None;
        }
        slot.dirty = false;
        slot.published = Some(now);
        Some(StatusChange(slot.current.clone()))
    }
}

/// A status to publish; `None` clears it.
#[derive(Debug, PartialEq, Eq)]
pub struct StatusChange(pub Option<ProgramStatus>);

#[cfg(test)]
mod tests {
    use super::super::{Events, process};
    use super::StatusChange;
    use std::time::Instant;

    fn parser() -> vt100::Parser<Events> {
        vt100::Parser::new_with_callbacks(24, 80, 100, Events::default())
    }

    /// What would be published: a state label, `clear`, or `nothing`.
    fn take(p: &mut vt100::Parser<Events>) -> &'static str {
        match p.callbacks_mut().take_program_status(Instant::now(), true) {
            None => "nothing",
            Some(StatusChange(None)) => "clear",
            Some(StatusChange(Some(status))) => status.state.label(),
        }
    }

    #[test]
    fn blocked_and_done_raise_one_notice_each_and_working_none() {
        let mut p = parser();
        process(&mut p, b"\x1b]7501;state=working:app=brew\x1b\\");
        process(&mut p, b"\x1b]7501;state=blocked:kind=auth:app=brew\x07");
        process(&mut p, b"\x1b]7501;state=blocked:kind=auth:app=brew\x07");
        process(&mut p, b"\x1b]7501;state=done:app=brew\x07");
        let titles: Vec<String> = p
            .callbacks()
            .notices
            .iter()
            .map(|n| n.title.clone())
            .collect();
        assert_eq!(titles, ["brew is waiting (auth)", "brew is done"]);
        assert_eq!(take(&mut p), "done");
    }

    #[test]
    fn a_prompt_ends_working_but_keeps_done() {
        let mut p = parser();
        process(
            &mut p,
            b"\x1b]7501;state=working:progress=30\x07\x1b]133;A\x07",
        );
        assert_eq!(take(&mut p), "clear");
        process(&mut p, b"\x1b]7501;state=done\x07\x1b]133;A\x07");
        assert_eq!(take(&mut p), "done");
        process(&mut p, b"\x1b]7501;state=clear\x07");
        assert_eq!(take(&mut p), "clear");
    }

    #[test]
    fn queries_get_the_supported_states_and_bad_reports_change_nothing() {
        let mut p = parser();
        process(&mut p, b"\x1b]7501;?\x1b\\");
        assert!(p.callbacks().replies[0].starts_with("\x1b]7501;?:states=idle,working"));
        process(&mut p, b"\x1b]7501;state=working:progress=900\x07");
        process(&mut p, b"\x1b]7501;state=bogus\x07\x1b]7501\x07");
        let mut huge = b"\x1b]7501;state=done:msg=".to_vec();
        huge.extend(std::iter::repeat_n(b'A', 70 * 1024));
        huge.push(0x07);
        process(&mut p, &huge);
        assert_eq!(take(&mut p), "nothing");
        assert!(p.callbacks().notices.is_empty());
    }
}
