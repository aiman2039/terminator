//! Long Perforce actions (Sync, Submit, Shelve, Unshelve). Like the commit
//! job, the run is admitted on the root's workspace-op queue and then hands
//! the transfer to its own queue, so later reads in the root do not wait for
//! it. Its reply is an `Update::Workspace` for the same op.
use super::super::submit::Services;
use crate::{Update, perforce, workspace_ops};
use std::path::{Path, PathBuf};
use terminator_core::async_service::{CancellationToken, OperationContext, Policy};

/// The transfer takes as long as it takes, up to [`perforce::RUN_TIMEOUT`];
/// Cancel stops it, and the queued run must still reply.
fn run_context(root: &Path) -> OperationContext {
    let mut context = OperationContext::new(
        perforce::RUN_SUBSYSTEM,
        root.display().to_string(),
        Policy::OrderedMutation,
    );
    context.deadline = None;
    context
}

impl Services {
    pub(in crate::gui_services) fn perforce_run(
        &self,
        root: PathBuf,
        op: workspace_ops::Op,
        updates: &mut Vec<Update>,
    ) {
        let service = self.clone();
        let (run_root, run_op) = (root.clone(), op.clone());
        let submitted =
            self.handle()
                .submit(run_context(&root), CancellationToken::new(), async move {
                    let result = match &run_op {
                        workspace_ops::Op::PerforceRun(run) => {
                            perforce::perform(perforce::PerformInput {
                                service: &service,
                                cwd: &run_root,
                                op: run.op.clone(),
                                cancel: &run.cancel,
                                long: true,
                            })
                            .await
                            .map_err(|error| format!("{error:#}"))
                        }
                        _ => Err("Only a long Perforce action runs here".into()),
                    };
                    Ok(vec![Update::Workspace(run_root, run_op, result)])
                });
        if let Err(error) = submitted {
            updates.push(Update::Workspace(root, op, Err(error.to_string())));
        }
    }
}
