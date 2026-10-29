//! Field groups of SSH host Properties: config picker, connection, group
//! and sign-in key files.
use super::*;
use crate::vnc_ui::form;

/// An empty SSH project: no id, so saving adds it.
pub(crate) fn blank_project() -> RemoteProject {
    RemoteProject {
        id: String::new(),
        name: String::new(),
        ssh: SshOptions::default(),
        root: String::new(),
        layout: serde_json::Value::Null,
        group: None,
        forwards: Vec::new(),
        last_connected: 0,
    }
}

pub(crate) fn config_host_summary(host: &ConfigHost) -> String {
    let mut text = String::new();
    if !host.user.is_empty() {
        text.push_str(&host.user);
        text.push('@');
    }
    text.push_str(&host.hostname);
    if let Some(port) = host.port {
        text.push_str(&format!(":{port}"));
    }
    if let Some(jump) = &host.proxyjump {
        text.push_str(&format!(" via {jump}"));
    }
    text
}

/// "From SSH config" picker. Choosing an alias leaves user and port blank so
/// OpenSSH applies the configured `User`, `Port`, `ProxyJump` and keys.
pub(super) fn config_host_picker(
    ui: &mut egui::Ui,
    hosts: Option<&[ConfigHost]>,
    draft: &mut RemoteProject,
) {
    let selected = hosts.and_then(|h| h.iter().find(|h| h.alias == draft.ssh.host));
    let text = match (hosts, selected) {
        (None, _) => "Loading…".to_owned(),
        (Some([]), _) => "No hosts in ~/.ssh/config".to_owned(),
        (Some(_), Some(host)) => host.alias.clone(),
        (Some(_), None) => "Choose a host…".to_owned(),
    };
    let mut chosen = None;
    form::label(ui, "From SSH config");
    ui.add_enabled_ui(hosts.is_some_and(|h| !h.is_empty()), |ui| {
        egui::ComboBox::from_id_salt("ssh-config-host")
            .selected_text(text)
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for host in hosts.unwrap_or_default() {
                    let label = format!("{}  ·  {}", host.alias, config_host_summary(host));
                    if ui
                        .selectable_label(host.alias == draft.ssh.host, label)
                        .clicked()
                    {
                        chosen = Some(host.alias.clone());
                    }
                }
            });
    });
    if let Some(alias) = chosen {
        if draft.name.trim().is_empty() || draft.name == draft.ssh.host {
            draft.name.clone_from(&alias);
        }
        draft.ssh.host = alias;
        draft.ssh.username.clear();
        draft.ssh.port = None;
    }
}

/// Host and port on one row, then the user.
pub(super) fn connection_fields(
    ui: &mut egui::Ui,
    draft: &mut RemoteProject,
    config: Option<&ConfigHost>,
) {
    let port_hint = config
        .and_then(|h| h.port)
        .map_or_else(|| "22".to_owned(), |p| p.to_string());
    ui.horizontal(|ui| {
        let width = (ui.available_width() - form::NUMBER_WIDTH - 8.0).max(120.0);
        ui.vertical(|ui| {
            ui.set_width(width);
            form::label(ui, "Host or config alias");
            ui.add(
                appearance::singleline(&mut draft.ssh.host)
                    .hint_text("build.example.com")
                    .desired_width(width),
            );
        });
        ui.vertical(|ui| {
            form::label(ui, "Port");
            appearance::optional_port(ui, &mut draft.ssh.port, &port_hint);
        });
    });
    let user_hint = config.filter(|h| !h.user.is_empty()).map_or_else(
        || "From SSH config or local user".to_owned(),
        |h| h.user.clone(),
    );
    form::text(ui, "User", &mut draft.ssh.username, &user_hint);
}

/// A key file path; `~/` expands to the home directory, blank clears it.
fn path_field(ui: &mut egui::Ui, label: &str, value: &mut Option<PathBuf>, hint: &str) {
    let mut text = value
        .as_ref()
        .map_or_else(String::new, |p| p.to_string_lossy().into_owned());
    if form::text(ui, label, &mut text, hint).changed() {
        *value = expand_home(text.trim());
    }
}

fn expand_home(text: &str) -> Option<PathBuf> {
    if text.is_empty() {
        return None;
    }
    match (text.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => Some(PathBuf::from(home).join(rest)),
        _ => Some(PathBuf::from(text)),
    }
}

/// The group field plus a menu of groups already in use.
pub(super) fn group_field(ui: &mut egui::Ui, group: &mut Option<String>, groups: &[String]) {
    form::label(ui, "Group");
    let mut text = group.clone().unwrap_or_default();
    ui.horizontal(|ui| {
        let width = (ui.available_width() - appearance::TOOLBAR_BUTTON - 4.0).max(60.0);
        ui.add(
            appearance::singleline(&mut text)
                .hint_text("None")
                .desired_width(width),
        );
        ui.add_enabled_ui(!groups.is_empty(), |ui| {
            appearance::icon_menu_button(ui, "ChevronDown", |ui| {
                for name in groups {
                    if ui.button(name).clicked() {
                        text.clone_from(name);
                        ui.close();
                    }
                }
            });
        });
    });
    *group = (!text.is_empty()).then_some(text);
}

/// The Sign-in section: key files stay folded unless one is set.
pub(super) fn sign_in_fields(ui: &mut egui::Ui, ssh: &mut SshOptions) {
    form::section(ui, "Sign-in");
    form::hint(
        ui,
        "Keys and agents come from OpenSSH. Passwords and 2FA are typed in the SSH terminal.",
        "",
    );
    let custom = ssh.identity_file.is_some() || ssh.known_hosts_file.is_some();
    egui::CollapsingHeader::new("Key files")
        .id_salt("ssh-key-files")
        .default_open(custom)
        .show(ui, |ui| {
            path_field(
                ui,
                "Identity file",
                &mut ssh.identity_file,
                "~/.ssh/id_ed25519",
            );
            path_field(
                ui,
                "Known hosts file",
                &mut ssh.known_hosts_file,
                "~/.ssh/known_hosts",
            );
        });
}
