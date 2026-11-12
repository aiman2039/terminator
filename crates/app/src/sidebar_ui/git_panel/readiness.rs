//! The readiness line above the Commit buttons: what the commit takes
//! (`1 of 3 files staged  +2 −1`, or `Nothing staged` with Stage all), and
//! at its right end Create PR or the known pull request.
use super::*;
use crate::workspace_ops::CompareData;

/// The pull request of the shown branch that the app already read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KnownPr {
    pub(crate) root: PathBuf,
    pub(crate) number: u64,
    pub(crate) url: String,
}

fn known_id() -> egui::Id {
    egui::Id::unique("git-known-pr")
}

/// Shares the known pull request with the panel; the app calls it each
/// frame.
pub(crate) fn publish(ctx: &egui::Context, known: Option<KnownPr>) {
    ctx.data_mut(|data| {
        if let Some(known) = known {
            data.insert_temp(known_id(), known);
        } else {
            data.remove::<KnownPr>(known_id());
        }
    });
}

/// What the commit takes.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Readiness {
    pub(super) staged: usize,
    pub(super) total: usize,
    /// Lines of the staged set. Only when every staged file is staged
    /// whole: the refresh adds index and worktree counts per path.
    pub(super) lines: Option<(u32, u32)>,
    /// Changes Stage all would take (conflicts stay out).
    pub(super) stageable: usize,
}

pub(super) fn readiness(context: &ContextData) -> Readiness {
    let staged: Vec<&terminator_git::Change> = context
        .changes
        .iter()
        .filter(|change| change.in_group(terminator_git::GitGroup::Staged))
        .collect();
    let whole = staged
        .iter()
        .all(|change| change.status.as_bytes().get(1) == Some(&b' '));
    let lines = (!staged.is_empty() && whole && !context.stats.is_empty()).then(|| {
        staged.iter().fold((0u32, 0u32), |(a, d), change| {
            let (add, del) = context.stats.get(&change.path).copied().unwrap_or_default();
            (a.saturating_add(add), d.saturating_add(del))
        })
    });
    Readiness {
        staged: staged.len(),
        total: context.changes.len(),
        lines,
        stageable: context.changes.iter().filter(|c| !c.conflict()).count(),
    }
}

/// The link at the right end of the readiness line.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum PrLink {
    /// The branch has an upstream, carries commits the default branch
    /// lacks, and no pull request is known.
    Create,
    Known(u64, String),
    Hidden,
}

pub(super) fn pr_link(compare: Option<&CompareData>, known: Option<&KnownPr>) -> PrLink {
    if let Some(known) = known {
        return PrLink::Known(known.number, known.url.clone());
    }
    match compare {
        Some(compare) if compare.upstream.is_some() && compare.pr_ahead.unwrap_or(0) > 0 => {
            PrLink::Create
        }
        _ => PrLink::Hidden,
    }
}

impl GitPanelPass<'_, '_> {
    /// The readiness line; nothing when the tree is clean.
    pub(super) fn readiness_line(&mut self, ui: &mut egui::Ui) {
        let context = self.input.context;
        let known = ui
            .ctx()
            .data(|data| data.get_temp::<KnownPr>(known_id()))
            .filter(|known| context.root.as_deref() == Some(known.root.as_path()));
        let link = pr_link(self.input.compare, known.as_ref());
        if context.changes.is_empty() && link == PrLink::Hidden {
            return;
        }
        let ready = readiness(context);
        let theme = self.input.theme;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if ready.staged == 0 && ready.total > 0 {
                ui.label(
                    RichText::new("Nothing staged")
                        .small()
                        .color(appearance::color(&theme.status_waiting)),
                );
                self.stage_all_link(ui, ready.stageable);
            } else if ready.total > 0 {
                let noun = if ready.total == 1 { "file" } else { "files" };
                let text = format!("{} of {} {noun} staged", ready.staged, ready.total);
                ui.weak(RichText::new(text).small());
                if let Some((added, deleted)) = ready.lines {
                    let tint = |color: &str| appearance::color(color);
                    ui.label(
                        RichText::new(format!("+{added}"))
                            .small()
                            .color(tint(&theme.git_added)),
                    );
                    ui.label(
                        RichText::new(format!("−{deleted}"))
                            .small()
                            .color(tint(&theme.git_deleted)),
                    );
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.pr_link(ui, link);
            });
        });
    }

    fn stage_all_link(&mut self, ui: &mut egui::Ui, stageable: usize) {
        if stageable == 0 {
            return;
        }
        let lock = self.input.commit_lock.index;
        let link = appearance::enabled_unless(ui, lock, |ui| {
            ui.link(RichText::new(format!("Stage all ({stageable})")).small())
        });
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "git-stage-all-link", link.rect);
        if link.clicked() {
            self.outcome.stage_all = true;
        }
    }

    fn pr_link(&mut self, ui: &mut egui::Ui, link: PrLink) {
        match link {
            PrLink::Hidden => {}
            PrLink::Known(number, url) => {
                let open = ui
                    .link(RichText::new(format!("PR #{number}")).small())
                    .on_hover_text(&url);
                if open.clicked() {
                    self.outcome.open_url = Some(url);
                }
            }
            PrLink::Create => {
                let why = self
                    .input
                    .commit_run
                    .is_some()
                    .then_some(crate::workspace_ops::COMMIT_WAIT);
                let create = appearance::enabled_unless(ui, why, |ui| {
                    ui.link(RichText::new("Create PR").small())
                })
                .on_hover_text(
                    "Create a pull request (gh) or merge request (glab) for the pushed branch",
                );
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "git-create-pr", create.rect);
                if create.clicked() {
                    self.outcome.create_pr = true;
                }
            }
        }
    }
}
