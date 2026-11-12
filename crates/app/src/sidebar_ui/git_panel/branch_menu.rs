//! Branch and sync menu items: Fetch, Pull and Push in the ⋮ menu; New,
//! Rename, Merge, Rebase and Delete under the branch switcher; the commit
//! actions on History rows and in the Commit Log. A commit running in the
//! repository disables all but Copy hash ("Wait for the commit to finish").
use super::*;
use crate::git::branch_ui::{BranchDialog, BranchRequest};
use crate::workspace_ops::BranchOp;

/// What a commit's action menu picked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CommitPick {
    Copy(String),
    Branch(BranchRequest),
}

fn item(ui: &mut egui::Ui, label: &str, icon: &str, lock: Option<&str>) -> bool {
    let clicked = appearance::enabled_unless(ui, lock, |ui| {
        appearance::menu_item(appearance::MenuItemInput {
            ui,
            label,
            icon,
            shortcut: "",
        })
    })
    .clicked();
    if clicked {
        ui.close();
    }
    clicked
}

/// Fetch, Pull and Push (without a commit). Each runs on its click.
pub(super) fn sync_items(ui: &mut egui::Ui, lock: Option<&str>, outcome: &mut GitPanelOutcome) {
    for (label, icon, op) in [
        ("Fetch", "Download", BranchOp::Fetch),
        ("Pull", "ArrowDownToLine", BranchOp::Pull),
        ("Push", "Upload", BranchOp::Push),
    ] {
        if item(ui, label, icon, lock) {
            outcome.branch.push(BranchRequest::Run(op));
        }
    }
}

/// New, Rename, then Merge, Rebase and Delete with one branch each.
pub(super) fn branch_actions(
    ui: &mut egui::Ui,
    input: &GitPanelInput<'_>,
    current: &str,
    outcome: &mut GitPanelOutcome,
) {
    let lock = input.commit_lock.head;
    ui.separator();
    if item(ui, "New branch…", "Plus", lock) {
        outcome.branch.push(BranchRequest::Ask(BranchDialog::New {
            start: None,
            name: String::new(),
        }));
    }
    let detached = current.is_empty() || !input.branches.iter().any(|name| name == current);
    if !detached && item(ui, &format!("Rename {current}…"), "Pencil", lock) {
        outcome
            .branch
            .push(BranchRequest::Ask(BranchDialog::Rename {
                from: current.to_owned(),
                name: current.to_owned(),
            }));
    }
    let others: Vec<&String> = input.branches.iter().filter(|n| *n != current).collect();
    if others.is_empty() {
        return;
    }
    type Ask = fn(String) -> BranchDialog;
    let menus: [(&str, Ask); 3] = [
        ("Merge into current", BranchDialog::Merge),
        ("Rebase current onto", BranchDialog::Rebase),
        ("Delete branch", BranchDialog::Delete),
    ];
    for (title, ask) in menus {
        appearance::enabled_unless(ui, lock, |ui| {
            appearance::menu_button(ui, title, |ui| {
                for name in &others {
                    if item(ui, name, "GitBranch", None) {
                        outcome
                            .branch
                            .push(BranchRequest::Ask(ask((*name).clone())));
                    }
                }
            })
            .response
        });
    }
}

/// Copy hash, New branch, Checkout, Cherry-pick and Revert for `hash`.
pub(crate) fn commit_actions(
    ui: &mut egui::Ui,
    hash: &str,
    lock: Option<&str>,
) -> Option<CommitPick> {
    let mut pick = None;
    if item(ui, "Copy hash", "Copy", None) {
        pick = Some(CommitPick::Copy(hash.to_owned()));
    }
    let actions: [(&str, &str, BranchRequest); 4] = [
        (
            "New branch from here…",
            "Plus",
            BranchRequest::Ask(BranchDialog::New {
                start: Some(hash.to_owned()),
                name: String::new(),
            }),
        ),
        (
            "Checkout revision",
            "GitBranch",
            BranchRequest::Run(BranchOp::Checkout(hash.to_owned())),
        ),
        (
            "Cherry-pick",
            "GitCompareArrows",
            BranchRequest::Run(BranchOp::CherryPick(hash.to_owned())),
        ),
        (
            "Revert",
            "History",
            BranchRequest::Run(BranchOp::Revert(hash.to_owned())),
        ),
    ];
    for (label, icon, request) in actions {
        if item(ui, label, icon, lock) {
            pick = Some(CommitPick::Branch(request));
        }
    }
    pick
}
