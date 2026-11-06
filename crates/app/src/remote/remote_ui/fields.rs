//! Field groups of SSH host Properties: connection (with a config alias
//! menu), group and sign-in key files.
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
        command_timeout_seconds: None,
        mosh: false,
        shell_command: None,
        mosh_command: None,
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

/// Host and port on one row, then the user. The host field takes any name;
/// its menu lists `~/.ssh/config` aliases. Choosing an alias leaves user
/// and port blank so OpenSSH applies the configured `User`, `Port`,
/// `ProxyJump` and keys.
pub(super) fn connection_fields(
    ui: &mut egui::Ui,
    draft: &mut RemoteProject,
    hosts: Option<&[ConfigHost]>,
) {
    let config = hosts.and_then(|h| h.iter().find(|h| h.alias == draft.ssh.host));
    let port_hint = config
        .and_then(|h| h.port)
        .map_or_else(|| "22".to_owned(), |p| p.to_string());
    let user_hint = config.filter(|h| !h.user.is_empty()).map_or_else(
        || "From SSH config or local user".to_owned(),
        |h| h.user.clone(),
    );
    let mut chosen = None;
    ui.horizontal(|ui| {
        let menu = appearance::TOOLBAR_BUTTON + 4.0;
        let width = (ui.available_width() - form::NUMBER_WIDTH - menu - 8.0).max(120.0);
        ui.vertical(|ui| {
            ui.set_width(width + menu);
            form::label(ui, "Host or config alias");
            ui.horizontal(|ui| {
                ui.add(
                    appearance::singleline(&mut draft.ssh.host)
                        .hint_text("build.example.com")
                        .desired_width(width),
                );
                chosen = config_menu(ui, hosts);
            });
        });
        ui.vertical(|ui| {
            form::label(ui, "Port");
            appearance::optional_port(ui, &mut draft.ssh.port, &port_hint);
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
    form::text(ui, "User", &mut draft.ssh.username, &user_hint);
}

/// Optional menu of `~/.ssh/config` aliases; returns the chosen alias.
fn config_menu(ui: &mut egui::Ui, hosts: Option<&[ConfigHost]>) -> Option<String> {
    let mut chosen = None;
    let available = hosts.is_some_and(|h| !h.is_empty());
    let tip = match hosts {
        None => "Loading ~/.ssh/config…",
        Some([]) => "No hosts in ~/.ssh/config",
        Some(_) => "Choose a host from ~/.ssh/config (optional)",
    };
    ui.add_enabled_ui(available, |ui| {
        appearance::icon_menu_button(ui, "ChevronDown", |ui| {
            for host in hosts.unwrap_or_default() {
                let label = format!("{}  ·  {}", host.alias, config_host_summary(host));
                if ui.button(label).clicked() {
                    chosen = Some(host.alias.clone());
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text(tip)
        .on_disabled_hover_text(tip);
    });
    chosen
}

/// Per-host limit for one background SSH call; blank uses Settings → SSH.
/// `enabled` only when the daemon stores it (`ssh-host-timeout-v1`).
pub(super) fn timeout_field(
    ui: &mut egui::Ui,
    value: &mut Option<u32>,
    default: u32,
    enabled: bool,
) {
    form::label(ui, "Command timeout (seconds)");
    ui.add_enabled_ui(enabled, |ui| {
        let mut text = value.map_or_else(String::new, |v| v.to_string());
        let hint = format!("{default} (Settings → SSH)");
        let response = ui.add(
            appearance::singleline(&mut text)
                .hint_text(hint)
                .desired_width(160.0),
        );
        if response.changed() {
            let text = text.trim();
            if text.is_empty() {
                *value = None;
            } else if let Ok(seconds) = text.parse::<u32>() {
                *value = Some(seconds);
            }
        }
    })
    .response
    .on_disabled_hover_text("Requires an updated session service.");
    form::hint(
        ui,
        "Time for one background call (test, login directory, files, Git), sign-in included. 5–120 s. Increase it for slow or distant hosts.",
        "",
    );
}

/// Runs new terminals through Mosh. `enabled` only when the daemon honors
/// it (`ssh-mosh-v1`).
pub(super) fn mosh_field(ui: &mut egui::Ui, value: &mut bool, enabled: bool) {
    ui.add_enabled(
        enabled,
        egui::Checkbox::new(value, "Use Mosh for terminals"),
    )
    .on_disabled_hover_text("Requires an updated session service.");
    form::hint(
        ui,
        "Needs mosh on this computer and mosh-server on the host, with UDP ports 60000–61000 open. Files, Git and transfers still use SSH.",
        "",
    );
}

/// The terminal command, and the `mosh-server` command while Mosh is on.
/// `enabled` only when the daemon honors them (`ssh-commands-v1`).
pub(super) fn command_fields(ui: &mut egui::Ui, draft: &mut RemoteProject, enabled: bool) {
    ui.add_enabled_ui(enabled, |ui| {
        command_field(
            ui,
            "Terminal command",
            &mut draft.shell_command,
            terminator_core::remote::DEFAULT_SHELL_COMMAND,
        );
        form::hint(
            ui,
            "Runs on the host in the project folder for each new terminal, for example `tmux new -A -s main`. Editors do not use it.",
            "",
        );
        if draft.mosh {
            command_field(
                ui,
                "Mosh server command",
                &mut draft.mosh_command,
                terminator_core::remote::mosh::default_server_command(),
            );
            form::hint(
                ui,
                "Terminator adds `-- sh -c <terminal script>`. It must print MOSH CONNECT, as `mosh-server new` does.",
                "",
            );
        }
    })
    .response
    .on_disabled_hover_text("Requires an updated session service.");
}

/// An editable host command that shows `default` until changed. Blank or
/// the default saves nothing, so a later default still applies.
fn command_field(ui: &mut egui::Ui, label: &str, value: &mut Option<String>, default: &str) {
    form::label(ui, label);
    ui.horizontal(|ui| {
        let mut text = value.clone().unwrap_or_else(|| default.to_owned());
        let width = (ui.available_width() - 70.0).max(80.0);
        let response = ui.add(
            appearance::singleline(&mut text)
                .font(egui::TextStyle::Monospace)
                .desired_width(width),
        );
        if response.changed() {
            *value = terminator_core::remote::normalize_command(&text, default);
        }
        if ui
            .add_enabled(value.is_some(), egui::Button::new("Default"))
            .on_hover_text("Use the default command")
            .clicked()
        {
            *value = None;
        }
    });
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
