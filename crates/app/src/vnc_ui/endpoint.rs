use super::*;

pub(super) fn display_for_port(port: &str) -> String {
    port.trim()
        .parse::<u16>()
        .ok()
        .and_then(|port| port.checked_sub(terminator_core::vnc::DEFAULT_PORT))
        .map(|display| display.to_string())
        .unwrap_or_default()
}

pub(super) fn port_for_display(display: &str) -> String {
    display
        .trim()
        .parse::<u16>()
        .ok()
        .and_then(|display| display.checked_add(terminator_core::vnc::DEFAULT_PORT))
        .map(|port| port.to_string())
        .unwrap_or_default()
}

pub(super) fn port_display_fields(
    ui: &mut egui::Ui,
    port: &mut String,
    display: &mut String,
) -> bool {
    let port_response = ui
        .vertical(|ui| {
            ui.set_width(68.0);
            ui.label("Port");
            ui.add(appearance::singleline(port).desired_width(68.0))
                .on_hover_text("TCP port: 1–65535")
        })
        .inner;
    if port_response.changed() {
        *display = display_for_port(port);
    }
    let display_response = ui
        .vertical(|ui| {
            ui.set_width(68.0);
            ui.label("Display");
            ui.add(appearance::singleline(display).desired_width(68.0))
                .on_hover_text("Port = 5900 + display. Valid display: 0–59635")
        })
        .inner;
    if display_response.changed() {
        *port = port_for_display(display);
    }
    port_response.lost_focus() || display_response.lost_focus()
}

pub(super) fn connection_endpoint(host: &str, port: &str) -> Result<(String, u16)> {
    let port = port
        .trim()
        .parse::<u16>()
        .map_err(|_| anyhow::anyhow!("Port must be between 1 and 65535"))?;
    ensure!(port > 0, "Port must be between 1 and 65535");
    let host = host.trim();
    let host = host
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(host);
    ensure!(
        !host.contains(':') || host.parse::<std::net::Ipv6Addr>().is_ok(),
        "Enter only the host name or IP address in Host; use Port or Display for the connection number"
    );
    terminator_core::vnc::validate_host(host)?;
    ensure!(
        !host.trim().is_empty() && host.len() <= 255,
        "VNC host must be 1–255 characters"
    );
    Ok((host.to_owned(), port))
}
