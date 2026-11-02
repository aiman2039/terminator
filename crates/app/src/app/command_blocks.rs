//! Shell integration command list (`command-blocks-v1`): the main-pane page
//! that lists a terminal's commands, marks failures, copies one command's
//! output, and scrolls the terminal to a command.
use super::super::appearance;
use eframe::egui;
use terminator_core::command_blocks::{CAPABILITY, CommandBlock};
use terminator_core::*;

use super::super::*;

/// Row index (into `find` matches, oldest first) for the `nth_newest`
/// occurrence of a command line. `None` when it is no longer in the grid.
pub(crate) fn occurrence(matches: usize, nth_newest: usize) -> Option<usize> {
    matches.checked_sub(nth_newest.checked_add(1)?)
}

/// How many blocks newer than `blocks[index]` ran the same command line.
pub(crate) fn newer_same_command(blocks: &[CommandBlock], index: usize) -> usize {
    let Some(block) = blocks.get(index) else {
        return 0;
    };
    blocks
        .iter()
        .skip(index.saturating_add(1))
        .filter(|other| other.command == block.command)
        .count()
}

pub(crate) fn status_label(block: &CommandBlock) -> String {
    match (block.exit_code, block.duration_ms()) {
        (_, None) => "running".into(),
        (Some(code), Some(ms)) => format!("exit {code} · {}", duration_label(ms)),
        (None, Some(ms)) => duration_label(ms),
    }
}

pub(crate) fn duration_label(ms: u64) -> String {
    match ms {
        0..1_000 => format!("{ms} ms"),
        1_000..60_000 => format!("{:.1} s", ms as f64 / 1000.0),
        _ => format!("{}m {}s", ms / 60_000, (ms % 60_000) / 1000),
    }
}

impl App {
    /// Whether the session's daemon generation reports command blocks.
    pub(crate) fn command_blocks_supported(&self, session: &Session) -> bool {
        let current = || self.state.capabilities.iter().any(|c| c == CAPABILITY);
        if session.generation.is_empty() || session.generation == self.state.generation {
            return current();
        }
        self.state.generations.iter().any(|g| {
            g.owner.id == session.generation && g.capabilities.iter().any(|c| c == CAPABILITY)
        })
    }

    pub(crate) fn open_command_blocks(&mut self, sid: &str) {
        self.hide_center_overlay();
        self.commands_session = Some(sid.to_owned());
        self.request_command_blocks(sid);
    }

    pub(crate) fn close_command_blocks(&mut self) {
        self.commands_session = None;
    }

    fn request_command_blocks(&mut self, sid: &str) {
        let key = format!("commands:{sid}");
        if self.loading.insert(key) {
            let _ = self.jobs.send(Job::rpc(
                Request::CommandBlocks {
                    session: sid.to_owned(),
                    since: 0,
                },
                After::CommandBlocks(sid.to_owned()),
            ));
        }
    }

    pub(crate) fn handle_command_blocks(&mut self, sid: String, blocks: Vec<CommandBlock>) {
        self.loading.remove(&format!("commands:{sid}"));
        self.command_blocks.insert(sid, blocks.into());
    }

    /// Scroll the terminal to the command's line and show it.
    fn reveal_command(&mut self, sid: &str, index: usize) {
        let Some(blocks) = self.command_blocks.get(sid) else {
            return;
        };
        let Some(block) = blocks.get(index) else {
            return;
        };
        let newer = newer_same_command(blocks, index);
        let command = block.command.lines().next().unwrap_or_default().to_owned();
        self.go_session(sid);
        let Some(backend) = self.backends.get_mut(sid) else {
            return;
        };
        if command.trim().is_empty() {
            return;
        }
        let found = backend.find(&command, false);
        match occurrence(found.matches.len(), newer).and_then(|at| found.matches.get(at)) {
            Some(hit) => backend.reveal_grid_line(hit.line),
            None => self.info = Some("The command is no longer in the terminal".into()),
        }
    }

    pub(crate) fn command_blocks_center(&mut self, ui: &mut egui::Ui) {
        let Some(sid) = self.commands_session.clone() else {
            return;
        };
        ui.set_min_size(ui.available_size());
        ui.horizontal(|ui| {
            ui.strong("Commands");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if appearance::sidebar_action(ui, "X", "Close").clicked() {
                    self.close_command_blocks();
                }
                if appearance::sidebar_action(ui, "RefreshCw", "Refresh").clicked() {
                    self.request_command_blocks(&sid);
                }
            });
        });
        ui.add_space(6.0);
        ui.add(
            appearance::singleline(&mut self.search)
                .hint_text("Filter commands…")
                .desired_width(400.0),
        );
        ui.add_space(8.0);
        // Arc clone: no per-frame copy of block output (#174).
        let Some(blocks) = self.command_blocks.get(&sid).cloned() else {
            ui.weak("Loading commands…");
            return;
        };
        if blocks.is_empty() {
            ui.weak(
                "No commands yet. Commands appear after the shell reports them; \
                 restart the shell if it started before the update.",
            );
            return;
        }
        let failed = appearance::color(&self.theme.status_failed);
        let needle = self.search.to_lowercase();
        let mut reveal = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (index, block) in blocks.iter().enumerate().rev() {
                if !needle.is_empty() && !block.command.to_lowercase().contains(&needle) {
                    continue;
                }
                if self.command_row(ui, block, failed) {
                    reveal = Some(index);
                }
            }
        });
        if let Some(index) = reveal {
            self.reveal_command(&sid, index);
        }
    }
}

impl App {
    /// One command line; true when the user asked to show it.
    fn command_row(
        &mut self,
        ui: &mut egui::Ui,
        block: &CommandBlock,
        failed: egui::Color32,
    ) -> bool {
        let mut reveal = false;
        ui.horizontal(|ui| {
            let (icon, color) = if block.running() {
                ("…", ui.visuals().weak_text_color())
            } else if block.failed() {
                ("✗", failed)
            } else {
                ("✓", ui.visuals().text_color())
            };
            ui.colored_label(color, icon);
            let command = egui::RichText::new(&block.command).monospace();
            let command = if block.failed() {
                command.color(failed)
            } else {
                command
            };
            reveal = ui
                .add(egui::Label::new(command).sense(egui::Sense::click()))
                .on_hover_text("Show in terminal")
                .clicked();
            ui.weak(status_label(block));
            if let Some(cwd) = &block.cwd {
                ui.weak(cwd.display().to_string());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if appearance::sidebar_action(ui, "Copy", "Copy output").clicked() {
                    ui.ctx().copy_text(block.output.clone());
                    self.info = Some(if block.output_truncated {
                        "Copied the end of the output".into()
                    } else {
                        "Output copied".into()
                    });
                }
                if appearance::sidebar_action(ui, "Terminal", "Copy command").clicked() {
                    ui.ctx().copy_text(block.command.clone());
                }
            });
        });
        reveal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(command: &str) -> CommandBlock {
        CommandBlock {
            command: command.into(),
            ..CommandBlock::default()
        }
    }

    #[test]
    fn repeated_commands_reveal_their_own_occurrence() {
        let blocks = [block("ls"), block("make"), block("ls"), block("ls")];
        assert_eq!(newer_same_command(&blocks, 0), 2);
        assert_eq!(newer_same_command(&blocks, 3), 0);
        assert_eq!(newer_same_command(&blocks, 1), 0);
        // Three `ls` lines in the grid: the oldest block maps to the first.
        assert_eq!(occurrence(3, 2), Some(0));
        assert_eq!(occurrence(3, 0), Some(2));
        // Scrolled out of the grid: nothing to show.
        assert_eq!(occurrence(1, 2), None);
    }

    #[test]
    fn status_shows_exit_code_and_duration() {
        let mut done = block("x");
        done.started_ms = 1_000;
        done.finished_ms = Some(3_500);
        done.exit_code = Some(1);
        assert_eq!(status_label(&done), "exit 1 · 2.5 s");
        assert_eq!(status_label(&block("y")), "running");
        assert_eq!(duration_label(125_000), "2m 5s");
        assert_eq!(duration_label(12), "12 ms");
    }
}
