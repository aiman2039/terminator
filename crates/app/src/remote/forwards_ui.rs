//! Port forward controls shared by the Hosts sidebar and an SSH project's
//! Info view. Every OpenSSH call runs in the daemon through `Job::Forwards`.
use crate::{App, Job, appearance};
use eframe::egui;
use std::time::Instant;
use terminator_core::{
    Request,
    remote::{Forward, MAX_FORWARDS, RemoteProject, validate_forwards},
};

/// The add/edit form; ports stay text until saved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ForwardDraft {
    pub project: String,
    /// `None` adds a forward.
    pub index: Option<usize>,
    pub local_port: String,
    pub remote_host: String,
    pub remote_port: String,
    pub auto_start: bool,
}
impl ForwardDraft {
    fn new(project: &str) -> Self {
        Self {
            project: project.to_owned(),
            remote_host: "localhost".into(),
            ..Self::default()
        }
    }
    fn edit(project: &str, index: usize, forward: &Forward) -> Self {
        Self {
            project: project.to_owned(),
            index: Some(index),
            local_port: forward.local_port.to_string(),
            remote_host: forward.remote_host.clone(),
            remote_port: forward.remote_port.to_string(),
            auto_start: forward.auto_start,
        }
    }
    /// Parses and validates the form into a forward.
    pub(crate) fn forward(&self) -> anyhow::Result<Forward> {
        let port = |text: &str, what: &str| {
            text.trim()
                .parse::<u16>()
                .ok()
                .filter(|p| *p > 0)
                .ok_or_else(|| anyhow::anyhow!("{what} port must be between 1 and 65535"))
        };
        let forward = Forward {
            local_port: port(&self.local_port, "Local")?,
            remote_host: self.remote_host.trim().to_owned(),
            remote_port: port(&self.remote_port, "Remote")?,
            auto_start: self.auto_start,
        };
        forward.validate()?;
        Ok(forward)
    }
}

/// `forwards` with `forward` replacing entry `index` (or appended), checked
/// as a whole: no two forwards may share a local port.
pub(crate) fn edited_forwards(
    forwards: &[Forward],
    index: Option<usize>,
    forward: Forward,
) -> anyhow::Result<Vec<Forward>> {
    let mut list = forwards.to_vec();
    match index.and_then(|i| list.get_mut(i)) {
        Some(slot) => *slot = forward,
        None => list.push(forward),
    }
    validate_forwards(&list)?;
    Ok(list)
}

/// The (project, local port) a start or stop request toggles.
pub(crate) fn forward_key(request: &Request) -> Option<(String, u16)> {
    match request {
        Request::StartForward {
            project,
            local_port,
        }
        | Request::StopForward {
            project,
            local_port,
        } => Some((project.clone(), *local_port)),
        _ => None,
    }
}

impl App {
    pub(crate) fn forwarding_supported(&self) -> bool {
        self.connected
            && self
                .state
                .capabilities
                .iter()
                .any(|c| c == terminator_core::remote::FORWARDING_CAPABILITY)
    }

    /// Re-reads forward status while a view showing forwards is drawn.
    pub(crate) fn poll_forwards(&mut self, ctx: &egui::Context) {
        if !self.forwarding_supported() {
            return;
        }
        let hosts = &mut self.remote_ui.hosts;
        ctx.request_repaint_after(crate::hosts_ui::STATUS_INTERVAL);
        if hosts.forwards_pending
            || hosts
                .forwards_polled
                .is_some_and(|at| at.elapsed() < crate::hosts_ui::STATUS_INTERVAL)
        {
            return;
        }
        hosts.forwards_pending = true;
        hosts.forwards_polled = Some(Instant::now());
        let _ = self.jobs.send(Job::Forwards(Request::ForwardStatus));
    }

    pub(crate) fn apply_forwards(
        &mut self,
        key: Option<(String, u16)>,
        result: Result<terminator_core::remote::ForwardStatus, crate::remote_ui::Failure>,
    ) {
        let hosts = &mut self.remote_ui.hosts;
        match &key {
            Some(key) => {
                hosts.forward_busy.remove(key);
            }
            None => hosts.forwards_pending = false,
        }
        match result {
            Ok(status) => {
                if let Some((project, _)) = &key {
                    hosts.forward_errors.remove(project);
                }
                hosts.forwards = status;
            }
            Err(failure) => {
                if let Some((project, _)) = key {
                    hosts.forward_errors.insert(project, failure.text);
                }
            }
        }
    }

    fn toggle_forward(&mut self, project: &str, local_port: u16, start: bool) {
        let key = (project.to_owned(), local_port);
        let hosts = &mut self.remote_ui.hosts;
        if !hosts.forward_busy.insert(key) {
            return;
        }
        hosts.forward_errors.remove(project);
        let request = if start {
            Request::StartForward {
                project: project.to_owned(),
                local_port,
            }
        } else {
            Request::StopForward {
                project: project.to_owned(),
                local_port,
            }
        };
        let _ = self.jobs.send(Job::Forwards(request));
    }

    fn save_forwards(&mut self, project: &str, forwards: Vec<Forward>) {
        self.send(Request::SetRemoteProjectForwards {
            project: project.to_owned(),
            forwards,
        });
    }

    /// Forward rows with on/off toggles, plus add, edit and remove.
    pub(crate) fn forwards_section(&mut self, ui: &mut egui::Ui, project: &RemoteProject) {
        if !self.forwarding_supported() {
            ui.weak("Restart the session service to enable port forwarding.");
            return;
        }
        self.poll_forwards(ui.ctx());
        let id = project.id.as_str();
        self.forwards_header(ui, project);
        self.forward_error_row(ui, id);
        if project.forwards.is_empty() && self.remote_ui.hosts.forward_draft.is_none() {
            ui.weak("No forwards. Add one to reach a remote port at localhost.");
        }
        let mut clicks = ForwardClicks::default();
        for (index, forward) in project.forwards.iter().enumerate() {
            let row = ForwardRow { id, index, forward };
            self.forward_row(ui, &row, &mut clicks);
        }
        self.apply_forward_clicks(project, clicks);
        self.forward_form(ui, project);
    }

    fn forwards_header(&mut self, ui: &mut egui::Ui, project: &RemoteProject) {
        ui.horizontal(|ui| {
            ui.weak("Port forwards · 127.0.0.1 only");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let full = project.forwards.len() >= MAX_FORWARDS;
                let response = ui
                    .add_enabled_ui(!full, |ui| {
                        appearance::sidebar_action(ui, "Plus", "Add port forward")
                    })
                    .inner;
                if response.clicked() {
                    self.remote_ui.hosts.forward_draft = Some(ForwardDraft::new(&project.id));
                }
            });
        });
    }

    fn forward_error_row(&mut self, ui: &mut egui::Ui, id: &str) {
        let Some(error) = self.remote_ui.hosts.forward_errors.get(id).cloned() else {
            return;
        };
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(appearance::color(&self.theme.status_failed), error);
            if appearance::sidebar_action(ui, "X", "Dismiss").clicked() {
                self.remote_ui.hosts.forward_errors.remove(id);
            }
        });
    }

    fn forward_row(&self, ui: &mut egui::Ui, row: &ForwardRow<'_>, clicks: &mut ForwardClicks) {
        let ForwardRow { id, index, forward } = *row;
        let hosts = &self.remote_ui.hosts;
        let active = hosts.forwards.is_active(id, forward.local_port);
        let busy = hosts
            .forward_busy
            .contains(&(id.to_owned(), forward.local_port));
        let failure = hosts
            .forwards
            .failure(id, forward.local_port)
            .map(str::to_owned);
        ui.horizontal(|ui| {
            self.forward_label(ui, forward, active);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(!active && !busy, |ui| {
                    if appearance::sidebar_action(ui, "Trash", "Remove forward").clicked() {
                        clicks.remove = Some(index);
                    }
                    if appearance::sidebar_action(ui, "Pencil", "Edit forward").clicked() {
                        clicks.edit = Some(index);
                    }
                });
                if busy {
                    ui.add(egui::Spinner::new());
                } else if forward_toggle(ui, active).clicked() {
                    clicks.toggle = Some((forward.local_port, !active));
                }
            });
        });
        if let Some(failure) = failure {
            ui.colored_label(appearance::color(&self.theme.status_failed), failure);
        }
    }

    fn forward_label(&self, ui: &mut egui::Ui, forward: &Forward, active: bool) {
        let color = if active {
            appearance::color(&self.theme.status_running)
        } else {
            ui.visuals().weak_text_color()
        };
        ui.colored_label(color, "●")
            .on_hover_text(if active { "Listening" } else { "Stopped" });
        let mut label = forward.label();
        if forward.auto_start {
            label.push_str(" · auto");
        }
        ui.label(label).on_hover_text(if forward.auto_start {
            "Starts when a terminal for this project opens"
        } else {
            "Started manually"
        });
    }

    fn apply_forward_clicks(&mut self, project: &RemoteProject, clicks: ForwardClicks) {
        let ForwardClicks {
            toggle,
            remove,
            edit,
        } = clicks;
        let id = project.id.as_str();
        if let Some((port, start)) = toggle {
            self.toggle_forward(id, port, start);
        }
        if let Some(index) = remove {
            let mut list = project.forwards.clone();
            list.remove(index);
            self.save_forwards(id, list);
        }
        if let Some(index) = edit
            && let Some(forward) = project.forwards.get(index)
        {
            self.remote_ui.hosts.forward_draft = Some(ForwardDraft::edit(id, index, forward));
        }
    }

    fn forward_form(&mut self, ui: &mut egui::Ui, project: &RemoteProject) {
        let mut save = false;
        let mut cancel = false;
        let Some(draft) = &mut self.remote_ui.hosts.forward_draft else {
            return;
        };
        if draft.project != project.id {
            return;
        }
        egui::Grid::new(("forward-form", &project.id))
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("Local port");
                ui.add(
                    appearance::singleline(&mut draft.local_port)
                        .hint_text("8080")
                        .desired_width(80.0),
                );
                ui.end_row();
                ui.label("Remote host");
                ui.add(
                    appearance::singleline(&mut draft.remote_host)
                        .hint_text("localhost (as seen from the SSH host)")
                        .desired_width(160.0),
                );
                ui.end_row();
                ui.label("Remote port");
                ui.add(
                    appearance::singleline(&mut draft.remote_port)
                        .hint_text("80")
                        .desired_width(80.0),
                );
                ui.end_row();
            });
        ui.checkbox(&mut draft.auto_start, "Start when a terminal opens");
        ui.horizontal(|ui| {
            save = ui.button("Save").clicked();
            cancel = ui.button("Cancel").clicked();
        });
        if cancel {
            self.remote_ui.hosts.forward_draft = None;
            return;
        }
        if !save {
            return;
        }
        let result = draft
            .forward()
            .and_then(|forward| edited_forwards(&project.forwards, draft.index, forward));
        match result {
            Ok(list) => {
                self.remote_ui.hosts.forward_draft = None;
                self.save_forwards(&project.id, list);
            }
            Err(error) => {
                self.remote_ui
                    .hosts
                    .forward_errors
                    .insert(project.id.clone(), error.to_string());
            }
        }
    }
}

/// One configured forward of a project, at its list position.
struct ForwardRow<'a> {
    id: &'a str,
    index: usize,
    forward: &'a Forward,
}

/// Row buttons clicked this frame; the last row clicked wins.
#[derive(Default)]
struct ForwardClicks {
    /// Local port and whether to start it.
    toggle: Option<(u16, bool)>,
    remove: Option<usize>,
    edit: Option<usize>,
}

fn forward_toggle(ui: &mut egui::Ui, active: bool) -> egui::Response {
    appearance::selectable_icon(
        ui,
        if active { "CircleStop" } else { "Play" },
        if active {
            "Stop forward"
        } else {
            "Start forward (signs in with your key or agent, or reuses a signed-in terminal)"
        },
        active,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn forward(local_port: u16) -> Forward {
        Forward {
            local_port,
            remote_host: "localhost".into(),
            remote_port: 80,
            auto_start: false,
        }
    }

    #[test]
    fn forward_form_parses_ports_and_rejects_bad_input() {
        let mut draft = ForwardDraft::new("p");
        draft.local_port = " 8080 ".into();
        draft.remote_port = "5432".into();
        draft.remote_host = " db ".into();
        draft.auto_start = true;
        assert_eq!(
            draft.forward().unwrap(),
            Forward {
                local_port: 8080,
                remote_host: "db".into(),
                remote_port: 5432,
                auto_start: true,
            }
        );
        for (local, remote, host) in [
            ("0", "1", "db"),
            ("70000", "1", "db"),
            ("x", "1", "db"),
            ("1", "", "db"),
            ("1", "2", "-oProxyCommand=x"),
            ("1", "2", ""),
        ] {
            draft.local_port = local.into();
            draft.remote_port = remote.into();
            draft.remote_host = host.into();
            assert!(draft.forward().is_err(), "{local} {remote} {host}");
        }
    }

    #[test]
    fn editing_replaces_in_place_and_refuses_duplicate_local_ports() {
        let list = vec![forward(1), forward(2)];
        assert_eq!(
            edited_forwards(&list, Some(1), forward(3)).unwrap(),
            vec![forward(1), forward(3)]
        );
        assert_eq!(edited_forwards(&list, None, forward(3)).unwrap().len(), 3);
        assert!(edited_forwards(&list, None, forward(2)).is_err());
        assert!(edited_forwards(&list, Some(0), forward(2)).is_err());
        assert!(edited_forwards(&list, Some(1), forward(2)).is_ok());
    }

    #[test]
    fn only_start_and_stop_carry_a_toggle_key() {
        assert_eq!(
            forward_key(&Request::StartForward {
                project: "p".into(),
                local_port: 9,
            }),
            Some(("p".into(), 9))
        );
        assert_eq!(forward_key(&Request::ForwardStatus), None);
    }
}
