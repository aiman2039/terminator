//! Export of the flow plan: a worker thread writes `NAME.md` beside the
//! flow file, on an explicit click.
use super::FlowEditor;
use std::path::PathBuf;
use std::sync::mpsc::{TryRecvError, channel};
use terminator_core::flow::files;

impl FlowEditor {
    /// Export: a worker thread writes the plan as `NAME.md` beside the
    /// flow file. It replaces only that plan file.
    pub(super) fn export_plan(&mut self) {
        if self.exporting.is_some() {
            return;
        }
        let text = self.plan();
        let Ok(flow) = files::path_of(&self.root, &self.file) else {
            return;
        };
        let path = flow.with_extension("md");
        let (sender, receiver) = channel();
        let spawned = std::thread::Builder::new()
            .name("flow-export".into())
            .spawn(move || {
                let written = path
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| std::fs::write(&path, text));
                let _ = sender.send(written.map(|()| path).map_err(|error| error.to_string()));
            });
        if spawned.is_ok() {
            self.exporting = Some(receiver);
        } else {
            self.error = Some("Could not start the export.".into());
        }
    }

    /// Takes the result of an export: the written plan file.
    pub(super) fn poll_export(&mut self) -> Option<PathBuf> {
        match self.exporting.as_ref()?.try_recv() {
            Ok(Ok(path)) => {
                self.exporting = None;
                Some(path)
            }
            Ok(Err(error)) => {
                self.error = Some(format!("Export: {error}"));
                self.exporting = None;
                None
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.exporting = None;
                None
            }
        }
    }
}
