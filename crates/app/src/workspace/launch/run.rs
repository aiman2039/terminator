//! Running a launch file: one create request at a time. The first pane
//! of a tab opens a new top-level tab; each next pane splits the previous
//! one once that pane is in the tab. Commands wait for the prompt.
use super::model::{self, LaunchFile, Split};
use crate::*;
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

/// A step that waits longer than this ends the run.
const STEP_TIMEOUT: Duration = Duration::from_secs(30);

/// One create request of a launch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// The first pane of a tab: a new top-level tab with this id.
    Tab {
        id: String,
        name: Option<String>,
        cwd: Option<PathBuf>,
        post: Option<PostCreate>,
    },
    /// A split beside the previous pane of tab `tab`.
    Split {
        tab: String,
        split: Split,
        cwd: Option<PathBuf>,
        post: Option<PostCreate>,
    },
}

/// The steps of one file, in order. Pure: the trust check comes first.
pub(crate) fn plan(file: &LaunchFile, root: &Path, agent_words: &[String]) -> Vec<Step> {
    let mut steps = Vec::new();
    for tab in &file.tabs {
        let id = terminator_core::id();
        for (index, pane) in tab.panes.iter().enumerate() {
            let cwd = pane.cwd.as_ref().map(|cwd| root.join(cwd));
            let post = model::pane_input(pane, agent_words);
            steps.push(if index == 0 {
                Step::Tab {
                    id: id.clone(),
                    name: tab.name.clone(),
                    cwd,
                    post,
                }
            } else {
                Step::Split {
                    tab: id.clone(),
                    split: pane.split.unwrap_or_default(),
                    cwd,
                    post,
                }
            });
        }
    }
    steps
}

/// A launch in progress.
pub(crate) struct LaunchRun {
    project: String,
    steps: VecDeque<Step>,
    waiting: Option<Waiting>,
    /// The newest pane of the tab being built.
    last: Option<String>,
}

/// The step whose pane has not shown up in its tab yet.
struct Waiting {
    tab: String,
    /// The tab's name from the file, set when its first pane arrives.
    name: Option<String>,
    known: HashSet<String>,
    post: Option<PostCreate>,
    since: Instant,
}

impl App {
    /// Start a launch on an explicit Open click. A file from the
    /// repository needs the trust answer; this checks it again.
    pub(crate) fn start_launch(&mut self, project: &str, root: &Path, file: &LaunchFile) {
        if model::gate(true, self.lsp.trust_answer(root)) != model::Gate::Open {
            self.info = Some("Trust the project before opening its launch layout.".into());
            return;
        }
        if self.launch.run.is_some() {
            self.info = Some("A launch layout is still opening.".into());
            return;
        }
        if !self.project_available(project) {
            self.info = Some("The launch layout's project is not open.".into());
            return;
        }
        let words = crate::tasks::agent_words(&self.preferences.agent_presets);
        self.launch.run = Some(LaunchRun {
            project: project.to_owned(),
            steps: plan(file, root, &words).into(),
            waiting: None,
            last: None,
        });
        self.advance_launch();
    }

    /// Send the next create once the previous pane is in its tab. Runs
    /// every frame.
    pub(crate) fn advance_launch(&mut self) {
        let Some(mut run) = self.launch.run.take() else {
            return;
        };
        if let Some(waiting) = run.waiting.take() {
            match self.launch_arrival(&run.project, &waiting) {
                Some(sid) => {
                    if let Some(post) = waiting.post {
                        self.post_create.by_session.insert(sid.clone(), post);
                    }
                    if let Some(name) = waiting.name
                        && let Some(group) = self
                            .layouts
                            .get_mut(&run.project)
                            .and_then(|w| w.tabs.iter_mut().find(|g| g.id == waiting.tab))
                    {
                        group.name = Some(name);
                    }
                    run.last = Some(sid);
                }
                None if waiting.since.elapsed() > STEP_TIMEOUT => {
                    self.info = Some("The launch layout stopped: a terminal did not open.".into());
                    return;
                }
                None => {
                    run.waiting = Some(waiting);
                    self.launch.run = Some(run);
                    return;
                }
            }
        }
        let Some(step) = run.steps.pop_front() else {
            self.info = Some("Launch layout opened.".into());
            return;
        };
        run.waiting = self.send_launch_step(&run.project, run.last.as_deref(), step);
        self.launch.run = Some(run);
    }

    /// The new terminal of `waiting.tab`, if it arrived.
    fn launch_arrival(&self, project: &str, waiting: &Waiting) -> Option<String> {
        let group = self
            .layouts
            .get(project)?
            .tabs
            .iter()
            .find(|tab| tab.id == waiting.tab)?;
        group.layout.iter_all_tabs().find_map(|(_, tab)| match tab {
            Tab::Terminal(sid) if !waiting.known.contains(sid) => Some(sid.clone()),
            _ => None,
        })
    }

    fn launch_tab_terminals(&self, project: &str, tab: &str) -> HashSet<String> {
        self.layouts
            .get(project)
            .and_then(|w| w.tabs.iter().find(|group| group.id == tab))
            .map(|group| {
                group
                    .layout
                    .iter_all_tabs()
                    .filter_map(|(_, tab)| match tab {
                        Tab::Terminal(sid) => Some(sid.clone()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Send one create request; only new sessions, never a stop.
    fn send_launch_step(
        &mut self,
        project: &str,
        last: Option<&str>,
        step: Step,
    ) -> Option<Waiting> {
        let (tab, name, cwd, after, post) = match step {
            Step::Tab {
                id,
                name,
                cwd,
                post,
            } => (
                id.clone(),
                name,
                cwd,
                After::Workspace(id, Vec::new()),
                post,
            ),
            Step::Split {
                tab,
                split,
                cwd,
                post,
            } => {
                let anchor = last
                    .map(|sid| vec![Tab::Terminal(sid.to_owned())])
                    .unwrap_or_default();
                let split = Some(split.request().to_owned());
                let after = After::CreateAt(Some(tab.clone()), anchor, split);
                (tab, None, cwd, after, post)
            }
        };
        let request = Request::Create {
            project: project.to_owned(),
            cwd,
            file: None,
            line: None,
            column: None,
            editor: false,
        };
        let known = self.launch_tab_terminals(project, &tab);
        self.jobs.send(Job::rpc(request, after)).ok()?;
        Some(Waiting {
            tab,
            name,
            known,
            post,
            since: Instant::now(),
        })
    }
}
