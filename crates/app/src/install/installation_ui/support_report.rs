//! Settings → Updates → Installation details: Open logs folder, Copy
//! diagnostics and Report a problem. The copied text holds versions, OS,
//! capabilities and the daemon generation only: no scrollback, no settings,
//! and every path under the home folder shows as `~`.

use crate::*;
use std::path::{Path, PathBuf};

pub(crate) const ISSUES_URL: &str = "https://github.com/aiman2039/terminator/issues/new";

/// Pending "Report a problem…" preview, kept in egui memory so the
/// Settings page and the palette share it without new App state.
fn preview_id() -> egui::Id {
    egui::Id::unique("report-problem-preview")
}

pub(crate) struct ReportInput<'a> {
    pub gui_version: &'a str,
    pub daemon_version: Option<&'a str>,
    pub os: &'a str,
    pub arch: &'a str,
    pub generation: &'a str,
    pub capabilities: &'a [String],
    pub service: Option<&'a Path>,
    pub helper: Option<&'a Path>,
    pub data_dir: &'a Path,
    pub installation_error: Option<&'a str>,
}

/// Plain-text diagnostics with `home` redacted to `~`.
pub(crate) fn diagnostics_text(input: &ReportInput<'_>, home: Option<&Path>) -> String {
    let mut text = String::from("Terminator diagnostics\n");
    text.push_str(&format!("GUI: {}\n", input.gui_version));
    text.push_str(&format!(
        "Session service: {}\n",
        input.daemon_version.unwrap_or("not reported")
    ));
    text.push_str(&format!("OS: {} {}\n", input.os, input.arch));
    let generation = if input.generation.is_empty() {
        "not reported"
    } else {
        input.generation
    };
    text.push_str(&format!("Daemon generation: {generation}\n"));
    let mut capabilities = input.capabilities.to_vec();
    capabilities.sort();
    text.push_str(&format!("Capabilities: {}\n", capabilities.join(", ")));
    for (label, path) in [("Service", input.service), ("Helper", input.helper)] {
        if let Some(path) = path {
            text.push_str(&format!("{label}: {}\n", path.display()));
        }
    }
    text.push_str(&format!("Data folder: {}\n", input.data_dir.display()));
    if let Some(error) = input.installation_error {
        text.push_str(&format!("Installation error: {error}\n"));
    }
    match home.map(|home| home.display().to_string()) {
        Some(home) => redact_home(&text, &home),
        None => text,
    }
}

/// Replace each path that starts at `home` with `~`. A longer name that
/// only begins with the home path (`/home/al` in `/home/alice`) stays.
pub(crate) fn redact_home(text: &str, home: &str) -> String {
    let home = home.trim_end_matches(['/', '\\']);
    if home.is_empty() {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(home) {
        let Some((before, tail)) = rest.split_at_checked(at) else {
            break;
        };
        let Some(after) = tail.get(home.len()..) else {
            break;
        };
        out.push_str(before);
        let boundary = after
            .chars()
            .next()
            .is_none_or(|c| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '.')));
        out.push_str(if boundary { "~" } else { home });
        rest = after;
    }
    out.push_str(rest);
    out
}

/// The crash-log folder when it exists, else the data folder (it holds
/// `daemon.log`).
pub(crate) fn logs_folder(data: &Path, crashes_exist: bool) -> PathBuf {
    if crashes_exist {
        data.join("crashes")
    } else {
        data.to_path_buf()
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

impl App {
    pub(crate) fn diagnostics_report(&self) -> String {
        diagnostics_text(
            &ReportInput {
                gui_version: env!("CARGO_PKG_VERSION"),
                daemon_version: self.state.daemon_version.as_deref(),
                os: std::env::consts::OS,
                arch: std::env::consts::ARCH,
                generation: &self.state.generation,
                capabilities: &self.state.capabilities,
                service: self.state.daemon_executable.as_deref(),
                helper: self.state.attachment_helper_executable.as_deref(),
                data_dir: &self.paths.data,
                installation_error: self.installation_error.as_deref(),
            },
            home_dir().as_deref(),
        )
    }

    /// Opens the folder in the system file manager on the job worker.
    pub(crate) fn open_logs_folder(&mut self) {
        let crashes = self.paths.data.join("crashes");
        let folder = logs_folder(&self.paths.data, crashes.is_dir());
        if self
            .jobs
            .send(Job::Browser(folder.display().to_string()))
            .is_err()
        {
            self.error
                .set("Job worker disconnected. Reopen Terminator to retry.");
        }
    }

    pub(crate) fn copy_diagnostics(&mut self, ctx: &egui::Context) {
        ctx.copy_text(self.diagnostics_report());
        self.info = Some("Diagnostics copied. They hold no terminal output or settings.".into());
    }

    /// Copy the diagnostics and show them in Settings → Updates before
    /// the issue page opens.
    pub(crate) fn report_problem(&mut self, ctx: &egui::Context) {
        let report = self.diagnostics_report();
        ctx.copy_text(report.clone());
        ctx.data_mut(|data| data.insert_temp(preview_id(), report));
        self.open_installation_settings();
    }

    pub(crate) fn support_buttons(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Open logs folder")
                .on_hover_text("Crash logs, or the data folder with daemon.log")
                .clicked()
            {
                self.open_logs_folder();
            }
            if ui
                .button("Copy diagnostics")
                .on_hover_text("Versions, OS and capabilities. No terminal output or settings.")
                .clicked()
            {
                self.copy_diagnostics(ui.ctx());
            }
            if ui.button("Report a problem…").clicked() {
                self.report_problem(ui.ctx());
            }
        });
    }

    /// The copied report, shown before the issue page opens.
    pub(crate) fn report_preview(&mut self, ui: &mut egui::Ui) {
        let Some(report) = ui.data(|data| data.get_temp::<String>(preview_id())) else {
            return;
        };
        ui.add_space(8.0);
        ui.strong("Report a problem");
        ui.label("These diagnostics are on the clipboard. Paste them into the new issue.");
        egui::ScrollArea::vertical()
            .id_salt("report-problem-text")
            .max_height(160.0)
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(RichText::new(&report).monospace())
                        .wrap()
                        .selectable(true),
                );
            });
        let mut close = false;
        ui.horizontal(|ui| {
            if ui.button("Open GitHub issue").clicked() {
                if self.jobs.send(Job::Browser(ISSUES_URL.into())).is_err() {
                    self.error
                        .set("Job worker disconnected. Reopen Terminator to retry.");
                }
                close = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
        if close {
            ui.data_mut(|data| data.remove::<String>(preview_id()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(capabilities: &'a [String], data: &'a Path, service: &'a Path) -> ReportInput<'a> {
        ReportInput {
            gui_version: "1.2.3",
            daemon_version: Some("1.2.2"),
            os: "linux",
            arch: "x86_64",
            generation: "gen-7",
            capabilities,
            service: Some(service),
            helper: None,
            data_dir: data,
            installation_error: Some("helper missing at /home/alice/bin/terminator-hook"),
        }
    }

    #[test]
    fn diagnostics_redact_paths_under_home() {
        let capabilities = vec!["vnc-v1".to_string(), "ssh-edit-v1".to_string()];
        let data = Path::new("/home/alice/.local/share/terminator");
        let service = Path::new("/home/alice/bin/terminator-daemon");
        let text = diagnostics_text(
            &input(&capabilities, data, service),
            Some(Path::new("/home/alice")),
        );
        assert!(!text.contains("/home/alice"), "{text}");
        assert!(text.contains("Data folder: ~/.local/share/terminator"));
        assert!(text.contains("Service: ~/bin/terminator-daemon"));
        assert!(text.contains("helper missing at ~/bin/terminator-hook"));
        assert!(text.contains("GUI: 1.2.3"));
        assert!(text.contains("Session service: 1.2.2"));
        assert!(text.contains("OS: linux x86_64"));
        assert!(text.contains("Daemon generation: gen-7"));
        assert!(text.contains("Capabilities: ssh-edit-v1, vnc-v1"));
    }

    #[test]
    fn redaction_keeps_other_users_with_the_same_prefix() {
        assert_eq!(
            redact_home("/home/al/x /home/alice/y /home/al", "/home/al/"),
            "~/x /home/alice/y ~"
        );
        assert_eq!(redact_home(r"C:\Users\bo\data", r"C:\Users\bo"), r"~\data");
        assert_eq!(redact_home("/x", ""), "/x");
    }

    #[test]
    fn logs_folder_prefers_crash_logs() {
        let data = Path::new("/data");
        assert_eq!(logs_folder(data, true), Path::new("/data/crashes"));
        assert_eq!(logs_folder(data, false), Path::new("/data"));
    }
}
