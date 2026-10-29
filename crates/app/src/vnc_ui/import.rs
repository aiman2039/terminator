//! Import saved hosts from `~/.ssh/config` (as SSH tunnels) or from
//! RealVNC Viewer `.vnc` connection files. Files are read on a background
//! thread; the user picks which hosts to add.
use super::*;
use std::{
    path::PathBuf,
    sync::mpsc::{Receiver, channel},
};
use terminator_core::vnc::{DEFAULT_PORT, Transport};

/// One host found by an import.
pub(crate) struct Candidate {
    pub(crate) profile: VncProfile,
    pub(crate) selected: bool,
    /// Where it came from, shown next to the host.
    pub(crate) source: &'static str,
}

#[derive(Default)]
pub(crate) struct ImportState {
    pub(crate) candidates: Vec<Candidate>,
    /// VNC display for hosts from the SSH config, which do not say one.
    pub(crate) display: String,
    loading: Option<Receiver<Vec<Candidate>>>,
}

impl ImportState {
    fn start(&mut self, read: impl FnOnce() -> Vec<Candidate> + Send + 'static) {
        let (sender, receiver) = channel();
        let _ = std::thread::Builder::new()
            .name("vnc-import".into())
            .spawn(move || {
                let _ = sender.send(read());
            });
        self.loading = Some(receiver);
    }

    /// Hosts from `~/.ssh/config`, read in the background.
    pub(crate) fn load_ssh_config(&mut self) {
        self.start(|| {
            let Some(home) = std::env::var_os("HOME") else {
                return Vec::new();
            };
            let path = PathBuf::from(home).join(".ssh").join("config");
            std::fs::read_to_string(path)
                .map(|text| ssh_config_candidates(&text))
                .unwrap_or_default()
        });
    }

    /// Hosts from RealVNC `.vnc` files, read in the background.
    pub(crate) fn load_vnc_files(&mut self, paths: Vec<PathBuf>) {
        self.start(move || {
            paths
                .iter()
                .filter_map(|path| std::fs::read_to_string(path).ok())
                .filter_map(|text| vnc_file_candidate(&text))
                .collect()
        });
    }

    /// Take finished results; true while still reading.
    pub(crate) fn poll(&mut self, saved: &[VncProfile]) -> bool {
        let Some(receiver) = &self.loading else {
            return false;
        };
        let Ok(found) = receiver.try_recv() else {
            return true;
        };
        self.loading = None;
        for candidate in found {
            let known = saved
                .iter()
                .chain(self.candidates.iter().map(|c| &c.profile))
                .any(|p| same_host(p, &candidate.profile));
            if !known {
                self.candidates.push(candidate);
            }
        }
        false
    }

    /// The selected hosts, ready to save. SSH hosts get the chosen display.
    pub(crate) fn selected_profiles(&self) -> Vec<VncProfile> {
        let display = self.display.trim().parse::<u16>().unwrap_or(0);
        let port = DEFAULT_PORT.saturating_add(display);
        self.candidates
            .iter()
            .filter(|c| c.selected)
            .map(|c| {
                let mut profile = c.profile.clone();
                if profile.options.transport == Transport::Ssh {
                    profile.port = port;
                }
                profile.id = id();
                profile
            })
            .collect()
    }
}

fn same_host(a: &VncProfile, b: &VncProfile) -> bool {
    let key = |p: &VncProfile| {
        if p.options.transport == Transport::Ssh {
            p.options.ssh.host.to_lowercase()
        } else {
            p.host.to_lowercase()
        }
    };
    a.options.transport == b.options.transport && key(a) == key(b)
}

fn blank_profile(name: &str, host: &str, port: u16) -> VncProfile {
    VncProfile {
        id: String::new(),
        name: name.to_owned(),
        host: host.to_owned(),
        port,
        username: String::new(),
        password: String::new(),
        shared: true,
        view_only: false,
        last_connected: 0,
        options: Default::default(),
        credential_saved: false,
        labels: Vec::new(),
    }
}

/// Concrete `Host` names (no wildcards or negations) as SSH-tunnel hosts.
pub(super) fn ssh_config_candidates(text: &str) -> Vec<Candidate> {
    let mut names: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some((keyword, rest)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        if !keyword.eq_ignore_ascii_case("host") {
            continue;
        }
        for name in rest.split_whitespace() {
            let concrete = !name.contains(['*', '?', '!']);
            if concrete && !names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
                names.push(name.to_owned());
            }
        }
    }
    names
        .into_iter()
        .map(|name| {
            let mut profile = blank_profile(&name, "localhost", DEFAULT_PORT);
            profile.options.transport = Transport::Ssh;
            profile.options.ssh.host.clone_from(&name);
            profile.options.reconnect.enabled = true;
            Candidate {
                profile,
                selected: false,
                source: "SSH config",
            }
        })
        .collect()
}

/// A RealVNC `.vnc` file: `Host=host:display` (RealVNC: a number below 100
/// is a display, otherwise a port; `host::port` is always a port) and an
/// optional `FriendlyName=`.
pub(super) fn vnc_file_candidate(text: &str) -> Option<Candidate> {
    let value = |key: &str| {
        text.lines().find_map(|line| {
            let (k, v) = line.trim().split_once('=')?;
            k.trim()
                .eq_ignore_ascii_case(key)
                .then(|| v.trim().to_owned())
        })
    };
    let address = value("Host").filter(|h| !h.is_empty())?;
    let (host, port) = match address.split_once("::") {
        Some((host, port)) => (host.to_owned(), port.parse::<u16>().ok()?),
        None => match address.rsplit_once(':') {
            Some((host, number)) => {
                let number = number.parse::<u16>().ok()?;
                let port = if number < 100 {
                    DEFAULT_PORT.saturating_add(number)
                } else {
                    number
                };
                (host.to_owned(), port)
            }
            None => (address.clone(), DEFAULT_PORT),
        },
    };
    let name = value("FriendlyName").unwrap_or_default();
    Some(Candidate {
        profile: blank_profile(&name, &host, port),
        selected: true,
        source: "RealVNC",
    })
}

impl App {
    /// The import checklist, shown on the address-book page in place of
    /// the cards while an import is open.
    pub(super) fn vnc_import_panel(&mut self, ui: &mut egui::Ui) {
        let saved = self.state.vnc_profiles.clone();
        let Some(import) = self.vnc_sidebar.import.as_mut() else {
            return;
        };
        if import.poll(&saved) {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        let mut close = false;
        let mut pick_files = false;
        let mut save = Vec::new();
        form::card(ui, |ui| {
            ui.label(RichText::new("Import hosts").size(16.0).strong());
            ui.horizontal(|ui| {
                if ui.button("From ~/.ssh/config").clicked() {
                    import.load_ssh_config();
                }
                pick_files = ui.button("RealVNC files…").clicked();
            });
            import_list(ui, import);
            ui.add_space(8.0);
            ui.separator();
            let count = import.candidates.iter().filter(|c| c.selected).count();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = format!("Import {count}");
                if ui
                    .add_enabled(count > 0, egui::Button::new(label))
                    .clicked()
                {
                    save = import.selected_profiles();
                    close = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        for profile in save {
            self.send(Request::VncProfileSave {
                profile,
                forget_password: true,
            });
        }
        if pick_files {
            self.pick_vnc_files = true;
        }
        if close {
            self.vnc_sidebar.import = None;
        }
    }
}

/// Found hosts with checkboxes, and the display for SSH-config hosts.
fn import_list(ui: &mut egui::Ui, import: &mut ImportState) {
    form::section(ui, "Found hosts");
    if import.candidates.is_empty() {
        form::hint(
            ui,
            "Choose a source above. Hosts you already saved are left out.",
            "",
        );
        return;
    }
    egui::ScrollArea::vertical()
        .max_height(320.0)
        .show(ui, |ui| {
            for candidate in &mut import.candidates {
                let text = format!(
                    "{}  ·  {}",
                    candidate.profile.display_name(),
                    candidate.source
                );
                ui.checkbox(&mut candidate.selected, text);
            }
        });
    if import
        .candidates
        .iter()
        .any(|c| c.profile.options.transport == Transport::Ssh)
    {
        ui.add_space(4.0);
        form::text(
            ui,
            "VNC display on SSH-config hosts",
            &mut import.display,
            "0 (port 5900)",
        );
    }
}

impl App {
    /// Picked RealVNC files: read them into the open import panel.
    pub(crate) fn vnc_import_files(&mut self, paths: Vec<PathBuf>) {
        self.picker_active = false;
        if paths.is_empty() {
            return;
        }
        self.vnc_sidebar
            .import
            .get_or_insert_with(ImportState::default)
            .load_vnc_files(paths);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_config_hosts_skip_patterns_and_repeats() {
        let found = ssh_config_candidates(
            "Host dc6.example.com\n  User me\nHost * !bad\nHost build lab?\nhost DC6.example.com\n",
        );
        let names: Vec<_> = found
            .iter()
            .map(|c| c.profile.options.ssh.host.as_str())
            .collect();
        assert_eq!(names, vec!["dc6.example.com", "build"]);
        assert_eq!(
            found.first().map(|c| c.profile.options.transport),
            Some(Transport::Ssh)
        );
    }

    #[test]
    fn realvnc_files_read_displays_ports_and_names() {
        let one = vnc_file_candidate("[Connection]\nHost=dc6.example.com:92\nFriendlyName=RENO\n")
            .unwrap();
        assert_eq!(
            (one.profile.host.as_str(), one.profile.port),
            ("dc6.example.com", 5992)
        );
        assert_eq!(one.profile.name, "RENO");
        let port = vnc_file_candidate("Host=box::5901").unwrap();
        assert_eq!(port.profile.port, 5901);
        let high = vnc_file_candidate("Host=box:7939").unwrap();
        assert_eq!(high.profile.port, 7939);
        assert!(vnc_file_candidate("Name=x").is_none());
    }

    #[test]
    fn imported_ssh_hosts_take_the_chosen_display() {
        let mut state = ImportState {
            candidates: ssh_config_candidates("Host dc6\n"),
            display: "2039".into(),
            ..ImportState::default()
        };
        if let Some(candidate) = state.candidates.first_mut() {
            candidate.selected = true;
        }
        let profiles = state.selected_profiles();
        let profile = profiles.first().expect("one imported host");
        assert_eq!(profiles.len(), 1);
        assert_eq!(profile.port, 7939);
        assert!(!profile.id.is_empty());
    }
}
