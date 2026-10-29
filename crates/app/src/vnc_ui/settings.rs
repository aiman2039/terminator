//! Connection settings are saved per host and enforced by the daemon.
use super::*;
use terminator_core::vnc::{ClipboardMode, Quality, Transport, VncOptions};

/// Transport and authentication, the Properties "Security" page.
/// `supported`: the running daemon advertises the VNC features capability.
pub(super) fn security(ui: &mut egui::Ui, value: &mut VncOptions, supported: bool) {
    ui.add_enabled_ui(supported, |ui| {
        form::section(ui, "Connect through");
        let before = value.transport;
        form::segmented(
            ui,
            &mut value.transport,
            &[
                (Transport::Ssh, "SSH tunnel"),
                (Transport::Tls, "TLS"),
                (Transport::Direct, "Direct"),
            ],
        );
        if before != Transport::Ssh && value.transport == Transport::Ssh {
            enable_ssh_defaults(value);
        }
        ui.add_space(4.0);
        match value.transport {
            Transport::Direct => {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    RichText::new("Not encrypted: screen, keys and clipboard are sent in clear.")
                        .size(11.0),
                );
            }
            Transport::Tls => tls(ui, value),
            Transport::Ssh => ssh(ui, value),
        }
        form::section(ui, "Authentication");
        ui.checkbox(
            &mut value.allow_no_auth,
            "Allow a server without authentication",
        );
    });
    unsupported_hint(ui, supported);
}

/// Display, input and clipboard, the Properties "Options" page.
pub(super) fn options(ui: &mut egui::Ui, value: &mut VncOptions, supported: bool) {
    ui.add_enabled_ui(supported, |ui| display(ui, value));
    unsupported_hint(ui, supported);
}

fn unsupported_hint(ui: &mut egui::Ui, supported: bool) {
    if !supported {
        form::hint(
            ui,
            "Restart the session service to enable these options.",
            "",
        );
    }
}

fn tls(ui: &mut egui::Ui, value: &mut VncOptions) {
    form::text(
        ui,
        "Certificate name",
        &mut value.tls_server_name,
        "Blank uses the VNC host",
    );
    optional_path(
        ui,
        "CA certificate file",
        &mut value.tls_ca_file,
        "Blank uses OS trust",
    );
    form::hint(
        ui,
        "A valid certificate is required.",
        "TLS does not fall back to Direct VNC.",
    );
}

fn ssh(ui: &mut egui::Ui, value: &mut VncOptions) {
    ui.horizontal(|ui| {
        let spacing = ui.spacing().item_spacing.x;
        let width = (ui.available_width() - form::NUMBER_WIDTH - spacing).max(80.0);
        ui.vertical(|ui| {
            ui.set_width(width);
            form::label(ui, "SSH host");
            ui.add(
                appearance::singleline(&mut value.ssh.host)
                    .hint_text("Same as the address")
                    .desired_width(width),
            );
        });
        ui.vertical(|ui| {
            ui.set_width(form::NUMBER_WIDTH);
            form::label(ui, "SSH port");
            appearance::optional_port(ui, &mut value.ssh.port, "Config");
        });
    });
    form::text(
        ui,
        "SSH user",
        &mut value.ssh.username,
        "Blank uses SSH config",
    );
    form::text(
        ui,
        "Start command on host (optional)",
        &mut value.ssh_start_command,
        "~/bin/vnc2039",
    )
    .on_hover_text(
        "Runs over SSH before each connection, for example to start the VNC server. It must exit 0.",
    );
    egui::CollapsingHeader::new("SSH keys")
        .default_open(value.ssh.identity_file.is_some() || value.ssh.known_hosts_file.is_some())
        .show(ui, |ui| {
            optional_path(
                ui,
                "Identity file",
                &mut value.ssh.identity_file,
                "Optional",
            );
            optional_path(
                ui,
                "Known hosts file",
                &mut value.ssh.known_hosts_file,
                "Optional",
            );
        });
}

fn display(ui: &mut egui::Ui, value: &mut VncOptions) {
    form::section(ui, "Display");
    form::label(ui, "Image quality");
    form::segmented(
        ui,
        &mut value.quality,
        &[
            (Quality::Auto, "Automatic"),
            (Quality::LowBandwidth, "Low bandwidth"),
            (Quality::Sharp, "Sharp text"),
        ],
    );
    ui.checkbox(
        &mut value.remote_resize,
        "Resize remote desktop to fit the pane",
    );
    form::section(ui, "Clipboard");
    form::segmented(
        ui,
        &mut value.clipboard,
        &[
            (ClipboardMode::Both, "Both ways"),
            (ClipboardMode::SendOnly, "Send"),
            (ClipboardMode::ReceiveOnly, "Receive"),
            (ClipboardMode::Disabled, "Off"),
        ],
    );
    form::section(ui, "Connection");
    ui.horizontal(|ui| {
        ui.checkbox(
            &mut value.reconnect.enabled,
            "Reconnect after a network drop",
        );
        if value.reconnect.enabled {
            ui.add(egui::DragValue::new(&mut value.reconnect.max_attempts).range(1..=10))
                .on_hover_text(
                    "Maximum retries. Retries stop on authentication, SSH, or certificate errors.",
                );
        }
    });
    form::hint(ui, "Changes apply when you connect again.", "");
}

/// Defaults for a host that just switched to an SSH tunnel: tunnels drop on
/// sleep and network changes, so reconnect is on. The user can turn it off.
pub(super) fn enable_ssh_defaults(value: &mut VncOptions) {
    value.reconnect.enabled = true;
}

fn optional_path(
    ui: &mut egui::Ui,
    label: &str,
    path: &mut Option<std::path::PathBuf>,
    hint: &str,
) {
    let mut text = path
        .as_ref()
        .map_or_else(String::new, |p| p.to_string_lossy().into_owned());
    if form::text(ui, label, &mut text, hint).changed() {
        *path = (!text.trim().is_empty()).then(|| text.trim().into());
    }
}
