//! Branch and sync actions from the Git panel and the Commit Log. Fetch,
//! Pull, Push and the commit actions run on their click; New, Rename,
//! Delete, Merge and Rebase first ask in a dialog. Every one goes through
//! `queue_workspace`, so a running commit refuses it (`CommitRisk::Head`)
//! and its controls say "Wait for the commit to finish".
use crate::workspace_ops::{BranchOp, CommitRisk, NOT_MERGED, Op};
use crate::*;
use std::path::{Path, PathBuf};

/// A branch action that asks first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BranchDialog {
    /// A new branch at `start` (a commit hash; `HEAD` when `None`).
    New {
        start: Option<String>,
        name: String,
    },
    Rename {
        from: String,
        name: String,
    },
    Delete(String),
    /// `git branch -d` refused: the branch is not fully merged.
    ForceDelete(String),
    Merge(String),
    Rebase(String),
}

/// What a Git panel or Commit Log control asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BranchRequest {
    /// Runs at once: Fetch, Pull, Push, Checkout, Cherry-pick, Revert.
    Run(BranchOp),
    Ask(BranchDialog),
}

/// A dialog tied to the repository it was asked in: switching projects
/// drops it, so an answer never runs in another repository.
#[derive(Clone, Debug)]
pub(crate) struct PendingBranchDialog {
    root: PathBuf,
    dialog: BranchDialog,
}

impl BranchDialog {
    /// Title, body, and detail lines.
    fn text(&self, current: &str) -> (&'static str, String, &'static str) {
        let short = |hash: &str| hash.get(..7).unwrap_or(hash).to_owned();
        match self {
            Self::New { start: None, .. } => ("New branch", format!("From {current}"), ""),
            Self::New {
                start: Some(start), ..
            } => ("New branch", format!("From commit {}", short(start)), ""),
            Self::Rename { from, .. } => ("Rename branch", format!("Rename {from}"), ""),
            Self::Delete(name) => ("Delete branch", format!("Delete branch {name}?"), ""),
            Self::ForceDelete(name) => (
                "Force delete branch",
                format!("{name} is not fully merged."),
                "Force delete drops the commits that only this branch has.",
            ),
            Self::Merge(name) => (
                "Merge",
                format!("Merge {name} into {current}?"),
                "A conflict stops the merge; resolve it in the Git panel.",
            ),
            Self::Rebase(name) => (
                "Rebase",
                format!("Rebase {current} onto {name}?"),
                "A conflict stops the rebase; continue or abort it in a terminal.",
            ),
        }
    }

    /// The typed name, for the dialogs that take one.
    fn name_mut(&mut self) -> Option<&mut String> {
        match self {
            Self::New { name, .. } | Self::Rename { name, .. } => Some(name),
            _ => None,
        }
    }

    /// The action the answer runs; `None` when a name is still missing.
    fn op(&self) -> Option<BranchOp> {
        let typed = |name: &str| (!name.trim().is_empty()).then(|| name.trim().to_owned());
        Some(match self {
            Self::New { start, name } => BranchOp::New {
                name: typed(name)?,
                start: start.clone(),
            },
            Self::Rename { from, name } => BranchOp::Rename {
                from: from.clone(),
                to: typed(name)?,
            },
            Self::Delete(name) => BranchOp::Delete {
                name: name.clone(),
                force: false,
            },
            Self::ForceDelete(name) => BranchOp::Delete {
                name: name.clone(),
                force: true,
            },
            Self::Merge(name) => BranchOp::Merge(name.clone()),
            Self::Rebase(name) => BranchOp::Rebase(name.clone()),
        })
    }

    /// Delete asks with a destructive button that Enter never presses.
    fn destructive(&self) -> Option<&'static str> {
        match self {
            Self::Delete(_) => Some("Delete"),
            Self::ForceDelete(_) => Some("Force delete"),
            _ => None,
        }
    }

    fn verb(&self) -> &'static str {
        match self {
            Self::New { .. } => "Create",
            Self::Rename { .. } => "Rename",
            Self::Merge(_) => "Merge",
            Self::Rebase(_) => "Rebase",
            Self::Delete(_) | Self::ForceDelete(_) => "Delete",
        }
    }
}

impl App {
    /// Runs or asks about a branch action; refused while a commit runs.
    pub(crate) fn branch_request(&mut self, request: BranchRequest) {
        if self.refuse_during_commit(self.git_root().as_deref(), CommitRisk::Head) {
            return;
        }
        match request {
            BranchRequest::Run(op) => {
                self.queue_workspace(Op::Branch(op));
            }
            BranchRequest::Ask(dialog) => {
                self.git_extras.dialog = self
                    .op_root()
                    .map(|root| PendingBranchDialog { root, dialog });
            }
        }
    }

    /// A branch action ended: lists reload; an unmerged branch that
    /// `git branch -d` kept asks about force delete.
    pub(crate) fn branch_op_finished(
        &mut self,
        root: &Path,
        op: &BranchOp,
        result: &Result<workspace_ops::Report, String>,
    ) {
        if let (BranchOp::Delete { name, force: false }, Err(error)) = (op, result)
            && error.contains(NOT_MERGED)
        {
            self.git_extras.dialog = Some(PendingBranchDialog {
                root: root.to_path_buf(),
                dialog: BranchDialog::ForceDelete(name.clone()),
            });
        }
        if self.op_root().as_deref() == Some(root) {
            self.queue_workspace(Op::Branches);
            self.queue_workspace(Op::Compare {
                base: self.git.base_ref.clone(),
            });
        }
    }

    /// The pending branch dialog, if any.
    pub(crate) fn show_git_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut pending) = self.git_extras.dialog.take() else {
            return;
        };
        if self.op_root().as_ref() != Some(&pending.root) {
            return;
        }
        let current = self
            .context
            .as_ref()
            .map(|context| context.branch.clone())
            .unwrap_or_default();
        let (title, body, detail) = pending.dialog.text(&current);
        let lock = self.commit_wait(self.git_root().as_deref(), CommitRisk::Head);
        let missing = pending.dialog.op().is_none().then_some("Enter a name");
        let window = self.popups.window(ctx, title);
        let mut dialog = appearance::Confirm::new(body, detail);
        let button =
            appearance::ConfirmButton::new(pending.dialog.verb()).disabled(lock.or(missing));
        match pending.dialog.destructive() {
            Some(label) => {
                dialog = dialog.destructive(appearance::ConfirmButton::new(label).disabled(lock));
            }
            None => dialog = dialog.default_action(button),
        }
        let answer = appearance::confirm(ctx, window, dialog, |ui| {
            if let Some(name) = pending.dialog.name_mut() {
                let field = ui.add(appearance::singleline(name).hint_text("Branch name"));
                // The name field takes the keys when nothing else has them.
                if ui.memory(|memory| memory.focused().is_none()) {
                    field.request_focus();
                }
            }
        });
        match answer {
            Some(appearance::ConfirmAnswer::Default | appearance::ConfirmAnswer::Destructive) => {
                if let Some(op) = pending.dialog.op() {
                    self.queue_workspace(Op::Branch(op));
                }
            }
            Some(appearance::ConfirmAnswer::Cancel) => {}
            None => self.git_extras.dialog = Some(pending),
        }
    }
}

#[cfg(test)]
#[path = "branch_ui_tests.rs"]
mod tests;
