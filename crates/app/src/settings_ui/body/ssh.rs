//! Settings → SSH: connection timing the daemon passes to OpenSSH
//! (`ssh-settings-v1`). Host-specific options stay in `~/.ssh/config`.
use crate::*;
use terminator_core::ssh::Tuning;

/// One numeric SSH setting.
struct TuningRow<'a> {
    label: &'static str,
    description: &'static str,
    value: &'a mut u32,
    range: std::ops::RangeInclusive<u32>,
    suffix: &'static str,
    /// The running daemon honors this row.
    enabled: bool,
}

/// Row labels and their search keywords, in row order.
const SEARCH: [(&str, &str); 4] = [
    ("Keep-alive", "keepalive serveraliveinterval idle drop"),
    (
        "Keep shared connection",
        "reuse controlpersist multiplex master",
    ),
    ("Connect timeout", "connecttimeout unreachable"),
    ("Command timeout", "timeout probe slow sign-in login"),
];

impl App {
    pub(super) fn ssh_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("SSH");
        ui.weak(
            "Applies to new SSH connections. Host names, users, keys and jumps come from ~/.ssh/config.",
        );
        let has = |name: &str| self.state.capabilities.iter().any(|c| c == name);
        let supported = has(terminator_core::ssh::TUNING_CAPABILITY);
        let command_timeout = has(terminator_core::ssh::COMMAND_TIMEOUT_CAPABILITY);
        let shown = SEARCH.map(|(label, keywords)| self.field_visible(label, keywords));
        let draft = &mut self.settings_draft;
        let rows = [
            TuningRow {
                label: "Keep-alive",
                description: "Seconds between keep-alive messages. Three missed replies end the connection.",
                value: &mut draft.ssh_keepalive_seconds,
                range: Tuning::KEEPALIVE_SECONDS,
                suffix: " s",
                enabled: true,
            },
            TuningRow {
                label: "Keep shared connection",
                description: "Minutes the shared connection stays open after its last terminal closes, so files, Git and new terminals connect without signing in again.",
                value: &mut draft.ssh_reuse_minutes,
                range: Tuning::REUSE_MINUTES,
                suffix: " min",
                enabled: true,
            },
            TuningRow {
                label: "Connect timeout",
                description: "Seconds to wait for a host before the connection fails.",
                value: &mut draft.ssh_connect_timeout_seconds,
                range: Tuning::CONNECT_TIMEOUT_SECONDS,
                suffix: " s",
                enabled: true,
            },
            TuningRow {
                label: "Command timeout",
                description: "Seconds one background SSH call (connection test, login directory, files, Git) may take, sign-in included. Increase it for slow or distant hosts.",
                value: &mut draft.ssh_command_timeout_seconds,
                range: Tuning::COMMAND_TIMEOUT_SECONDS,
                suffix: " s",
                enabled: command_timeout,
            },
        ];
        ui.add_enabled_ui(supported, |ui| {
            for (row, shown) in rows.into_iter().zip(shown) {
                if !shown {
                    continue;
                }
                settings_controls::settings_row(settings_controls::SettingsRowInput {
                    ui,
                    label: row.label,
                    description: row.description,
                    add_control: |ui| {
                        ui.add_enabled(
                            row.enabled,
                            egui::DragValue::new(row.value)
                                .range(row.range)
                                .suffix(row.suffix),
                        );
                    },
                });
            }
        })
        .response
        .on_disabled_hover_text("Requires an updated session service.");
        if ui
            .add_enabled(supported, egui::Button::new("Reset to defaults"))
            .clicked()
        {
            self.reset_ssh_settings();
        }
    }

    fn reset_ssh_settings(&mut self) {
        let defaults = Settings::default();
        let draft = &mut self.settings_draft;
        draft.ssh_keepalive_seconds = defaults.ssh_keepalive_seconds;
        draft.ssh_reuse_minutes = defaults.ssh_reuse_minutes;
        draft.ssh_connect_timeout_seconds = defaults.ssh_connect_timeout_seconds;
        draft.ssh_command_timeout_seconds = defaults.ssh_command_timeout_seconds;
    }
}
