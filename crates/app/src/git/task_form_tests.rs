//! The New task form: one needed input, the rest behind Advanced.
use crate::{App, CenterPage, Job, Paths, worktree_ui};
use eframe::egui;

fn target(ctx: &egui::Context, name: &str) -> Option<egui::Rect> {
    ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("fixture-target", name))))
}

struct Form {
    app: App,
    ctx: egui::Context,
    _dir: tempfile::TempDir,
    jobs: std::sync::mpsc::Receiver<Job>,
    time: f64,
}

impl Form {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
        let (jobs, requests) = std::sync::mpsc::channel();
        app.jobs = jobs.into();
        app.worktree_draft = Some(worktree_ui::WorktreeDraft {
            source: "a".into(),
            start: "HEAD".into(),
            branch: "terminator/a".into(),
            dest: "/src/terminator-a".into(),
            open_terminal: true,
            agent_command: "claude".into(),
            focus_name: true,
            ..Default::default()
        });
        app.show_center(CenterPage::WorktreeDraft);
        Self {
            app,
            ctx,
            _dir: dir,
            jobs: requests,
            time: 0.0,
        }
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.1;
        let input = egui::RawInput {
            time: Some(self.time),
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 700.0),
            )),
            events,
            ..Default::default()
        };
        let app = &mut self.app;
        let mut output = self.ctx.run_ui(input, |ui| app.worktree_center(ui));
        output.textures_delta.clear();
    }

    fn click(&mut self, name: &str) {
        let pos = target(&self.ctx, name)
            .unwrap_or_else(|| panic!("{name} is painted"))
            .center();
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.frame(vec![egui::Event::PointerMoved(pos)]);
        self.frame(vec![press(true)]);
        self.frame(vec![press(false)]);
        self.frame(vec![]);
    }

    fn creates(&self) -> usize {
        self.jobs
            .try_iter()
            .filter(|job| matches!(job, Job::CreateWorktree(..)))
            .count()
    }
}

#[test]
fn branch_and_folder_rows_wait_behind_advanced() {
    let mut form = Form::new();
    form.frame(vec![]);
    form.frame(vec![]);
    assert!(target(&form.ctx, "worktree-task-name").is_some());
    assert!(target(&form.ctx, "worktree-dest").is_none());
    form.click("worktree-advanced");
    assert!(target(&form.ctx, "worktree-dest").is_some());
}

#[test]
fn create_task_waits_for_a_task_name_and_types_without_enter() {
    let mut form = Form::new();
    form.frame(vec![]);
    form.frame(vec![]);
    // The name field has focus on open: typing goes straight into it.
    form.frame(vec![egui::Event::Text("fix".into())]);
    form.frame(vec![]);
    assert_eq!(form.app.worktree_draft.as_ref().unwrap().task, "fix");
    if let Some(draft) = form.app.worktree_draft.as_mut() {
        draft.task.clear();
    }
    form.click("worktree-create");
    assert_eq!(form.creates(), 0, "no task name, no Create");
    form.click("worktree-task-name");
    form.frame(vec![egui::Event::Text("fix-login".into())]);
    form.frame(vec![]);
    let draft = form.app.worktree_draft.clone().unwrap();
    assert_eq!(draft.task, "fix-login");
    form.click("worktree-create");
    let sent: Vec<Job> = form.jobs.try_iter().collect();
    let start = sent.iter().find_map(|job| match job {
        Job::CreateWorktree(draft) => Some(draft.task_start()),
        _ => None,
    });
    let start = start.expect("Create task sends the draft");
    assert_eq!(start.typed_input(), "claude");
}

#[test]
fn the_summary_names_branch_start_source_and_folder() {
    let draft = worktree_ui::WorktreeDraft {
        start: "HEAD".into(),
        branch: "terminator/fix".into(),
        ..Default::default()
    };
    assert_eq!(
        super::task_form::task_summary(&draft, "webapp", "terminator-fix"),
        "terminator/fix from HEAD of webapp in terminator-fix"
    );
}
