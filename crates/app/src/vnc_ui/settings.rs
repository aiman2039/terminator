//! Connection settings are saved per host and enforced by the daemon.
use super::*;
use terminator_core::vnc::{ClipboardMode, Quality, Transport, VncOptions};
pub(super) fn options(ui: &mut egui::Ui, value: &mut VncOptions, supported: bool) {
    ui.add_enabled_ui(supported,|ui|{
        egui::CollapsingHeader::new("Connection options").show(ui,|ui|{
            ui.horizontal_wrapped(|ui|{
                ui.label("Transport");
                ui.selectable_value(&mut value.transport,Transport::Direct,"Direct VNC");
                ui.selectable_value(&mut value.transport,Transport::Tls,"TLS");
                ui.selectable_value(&mut value.transport,Transport::Ssh,"SSH tunnel");
            });
            match value.transport{
                Transport::Direct=>{ui.weak("Direct VNC does not encrypt screen, keyboard, or clipboard traffic.");}
                Transport::Tls=>{
                    ui.label("TLS certificate name (blank uses the VNC host)");
                    ui.add(appearance::singleline(&mut value.tls_server_name).desired_width(f32::INFINITY));
                    optional_path(ui,"CA certificate file (blank uses OS trust)",&mut value.tls_ca_file);
                    ui.weak("A valid certificate is required. TLS does not fall back to Direct VNC.");
                }
                Transport::Ssh=>{
                    ui.label("SSH host");ui.add(appearance::singleline(&mut value.ssh.host).hint_text("Host name or SSH config alias").desired_width(f32::INFINITY));
                    ui.label("SSH port");ui.add(egui::DragValue::new(&mut value.ssh.port).range(1..=65535));
                    ui.label("SSH user");ui.add(appearance::singleline(&mut value.ssh.username).desired_width(f32::INFINITY));
                    optional_path(ui,"SSH identity file (optional)",&mut value.ssh.identity_file);
                    optional_path(ui,"Known hosts file (optional)",&mut value.ssh.known_hosts_file);
                    ui.weak("Uses OpenSSH config and agent. The host key must already be trusted. SFTP uses the same host.");
                }
            }
            ui.checkbox(&mut value.allow_no_auth,"Allow a VNC server without authentication");
            ui.checkbox(&mut value.reconnect.enabled,"Reconnect after a network interruption");
            if value.reconnect.enabled{
                ui.horizontal(|ui|{ui.label("Maximum retries");ui.add(egui::DragValue::new(&mut value.reconnect.max_attempts).range(1..=10));});
                ui.weak("Retries stop on authentication, SSH, or certificate errors.");
            }
            ui.label("Clipboard sharing");
            ui.horizontal_wrapped(|ui|{
                for (mode,label) in [(ClipboardMode::Both,"Both ways"),(ClipboardMode::SendOnly,"Send only"),(ClipboardMode::ReceiveOnly,"Receive only"),(ClipboardMode::Disabled,"Off")]{ui.selectable_value(&mut value.clipboard,mode,label);}
            });
            ui.label("Image quality");
            ui.horizontal_wrapped(|ui|{
                for (quality,label) in [(Quality::Auto,"Automatic"),(Quality::LowBandwidth,"Low bandwidth"),(Quality::Sharp,"Sharp text")]{ui.selectable_value(&mut value.quality,quality,label);}
            });
            ui.checkbox(&mut value.remote_resize,"Resize remote desktop to fit the pane");
            ui.checkbox(&mut value.show_remote_cursor,"Show the remote cursor");
            ui.weak("Connection options apply when you connect again.");
        });
    });
    if !supported {
        ui.weak("Restart the session service to enable advanced VNC options.");
    }
}
fn optional_path(ui: &mut egui::Ui, label: &str, path: &mut Option<std::path::PathBuf>) {
    ui.label(label);
    let mut text = path
        .as_ref()
        .map_or_else(String::new, |p| p.to_string_lossy().into_owned());
    if ui
        .add(appearance::singleline(&mut text).desired_width(f32::INFINITY))
        .changed()
    {
        *path = (!text.trim().is_empty()).then(|| text.trim().into());
    }
}
