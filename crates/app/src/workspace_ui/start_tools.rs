//! Start panel Setup rows for tools missing from PATH: the editor program
//! that embedded and terminal editors run (Neovim by default) and Git.
//! PATH is probed on a worker thread, once per program name.
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, channel};

/// A tool the start panel checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ToolCheck {
    pub(crate) program: String,
    pub(crate) note: &'static str,
    pub(crate) install: &'static str,
}

/// The tools to check for this editor mode and editor program.
pub(crate) fn tool_checks(
    mode: terminator_core::EditorMode,
    editor_program: &str,
) -> Vec<ToolCheck> {
    let mut checks = Vec::new();
    if matches!(
        mode,
        terminator_core::EditorMode::Embedded | terminator_core::EditorMode::Terminal
    ) {
        let program = editor_program.trim();
        let neovim = program.is_empty() || program == "nvim";
        checks.push(ToolCheck {
            program: if neovim { "nvim" } else { program }.to_owned(),
            note: if neovim {
                "Neovim is not on PATH; editor tabs need it (or pick the native editor in Settings)."
            } else {
                "The editor program is not on PATH."
            },
            install: if neovim { "brew install neovim" } else { "" },
        });
    }
    checks.push(ToolCheck {
        program: "git".into(),
        note: "Git is not on PATH; the Git tool window and worktrees need it.",
        install: if cfg!(target_os = "macos") {
            "xcode-select --install"
        } else {
            "apt install git"
        },
    });
    checks
}

/// PATH probe results by program name, filled in the background.
#[derive(Default)]
pub(crate) struct ToolProbes {
    found: HashMap<String, bool>,
    pending: HashMap<String, Receiver<bool>>,
}

impl ToolProbes {
    /// Whether `program` is on PATH: `None` until the probe answers. The
    /// first ask starts the probe on a worker thread.
    pub(crate) fn found(&mut self, program: &str) -> Option<bool> {
        if let Some(found) = self.found.get(program) {
            return Some(*found);
        }
        if let Some(receiver) = self.pending.get(program) {
            let found = receiver.try_recv().ok()?;
            self.pending.remove(program);
            self.found.insert(program.to_owned(), found);
            return Some(found);
        }
        let (sender, receiver) = channel();
        let name = program.to_owned();
        let spawned = std::thread::Builder::new()
            .name("tool-probe".into())
            .spawn(move || {
                let _ = sender.send(terminator_core::find_executable(&name).is_some());
            });
        if spawned.is_ok() {
            self.pending.insert(program.to_owned(), receiver);
        }
        None
    }

    /// Still waiting on a probe (the panel repaints until it answers).
    pub(crate) fn waiting(&self) -> bool {
        !self.pending.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use terminator_core::EditorMode;

    #[test]
    fn neovim_is_checked_only_for_editors_that_run_it() {
        let names = |mode, program: &str| -> Vec<String> {
            tool_checks(mode, program)
                .into_iter()
                .map(|check| check.program)
                .collect()
        };
        assert_eq!(names(EditorMode::Embedded, ""), ["nvim", "git"]);
        assert_eq!(names(EditorMode::Terminal, "hx"), ["hx", "git"]);
        assert_eq!(names(EditorMode::Native, ""), ["git"]);
    }

    #[test]
    fn missing_tool_probe_answers_off_the_calling_thread() {
        let mut probes = ToolProbes::default();
        let name = "no-such-tool-terminator-test";
        let first = probes.found(name);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut answer = first;
        while answer.is_none() && std::time::Instant::now() < deadline {
            std::thread::yield_now();
            answer = probes.found(name);
        }
        assert_eq!(answer, Some(false));
        assert!(!probes.waiting());
    }
}
