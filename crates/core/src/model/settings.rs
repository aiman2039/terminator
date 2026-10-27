use super::*;

impl Default for Settings {
    fn default() -> Self {
        Self {
            events: [
                AgentState::WaitingInput,
                AgentState::WaitingPermission,
                AgentState::Completed,
                AgentState::Failed,
            ]
            .into(),
            os_events: [
                AgentState::WaitingInput,
                AgentState::WaitingPermission,
                AgentState::Failed,
            ]
            .into(),
            dismissal: Dismissal::OnFocus,
            notifications_side: true,
            terminal_notifications: true,
            terminal_notifications_os: false,
            notification_sound: true,
            automatic_update_checks: true,
            ntfy_enabled: false,
            ntfy_channel: String::new(),
            ntfy_machine: String::new(),
            pr_metadata: false,
            editor_mode: EditorMode::Embedded,
            review_mode: ReviewMode::Native,
            editor_program: "nvim".into(),
            external_editor: if cfg!(target_os = "macos") {
                "open"
            } else {
                "xdg-open"
            }
            .into(),
            external_args: vec![],
            lsp_servers: std::collections::BTreeMap::new(),
            shell: String::new(),
            history_days: 30,
            session_mib: 50,
            total_mib: 2048,
            scrollback_lines: 10_000,
            font_size: 13.0,
            editor_close_timeout_secs: 1,
            diff_split_default: false,
            native_vim: true,
            inlay_hints: true,
            keybindings: Settings::default_keybindings(),
        }
    }
}
impl Settings {
    fn default_keybindings() -> std::collections::BTreeMap<String, String> {
        [
            ("new_terminal".into(), "command+T".into()),
            ("open_file".into(), "command+O".into()),
            ("split_up".into(), "command+alt+Up".into()),
            ("split_down".into(), "command+alt+D".into()),
            ("split_left".into(), "command+shift+Left".into()),
            ("split_right".into(), "command+shift+D".into()),
            ("next_pane".into(), "command+]".into()),
            ("select_all".into(), "command+A".into()),
            ("find_in_terminal".into(), "command+F".into()),
            ("search_scrollback".into(), "command+alt+F".into()),
            ("copy_working_directory".into(), "command+alt+C".into()),
            ("rename_terminal".into(), "command+shift+R".into()),
            ("close_session".into(), "command+W".into()),
            ("clear_scrollback".into(), "command+shift+K".into()),
            ("editor_save".into(), "command+S".into()),
            ("compare_disk".into(), "command+shift+Y".into()),
            ("toggle_left_sidebar".into(), "command+B".into()),
            ("toggle_right_sidebar".into(), "command+L".into()),
            ("toggle_ide_mode".into(), "command+E".into()),
            ("move_to_main".into(), "command+alt+M".into()),
            ("move_to_strip".into(), "command+alt+Down".into()),
            ("open_settings".into(), "command+,".into()),
            ("open_palette".into(), "command+P".into()),
            ("goto_symbol".into(), "command+alt+O".into()),
            ("file_structure".into(), "command+F12".into()),
            ("navigate_back".into(), "command+alt+Left".into()),
            ("navigate_forward".into(), "command+alt+Right".into()),
            ("recent_files".into(), "command+alt+E".into()),
            ("next_attention".into(), "command+shift+J".into()),
            ("new_browser".into(), "command+alt+B".into()),
            ("new_vnc".into(), "command+alt+V".into()),
        ]
        .into()
    }

    pub fn validate(&self) -> Result<()> {
        // Disabled ntfy accepts any channel/machine contents (including
        // leftovers) so turning the toggle off always saves. Enabling
        // requires a usable channel again, and delivery still re-validates.
        if self.ntfy_enabled {
            ensure!(
                (1..=128).contains(&self.ntfy_channel.len())
                    && self
                        .ntfy_channel
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
                "ntfy channel must contain 1–128 letters, digits, underscores or hyphens when enabled"
            );
            ensure!(
                self.ntfy_machine.len() <= 128 && !self.ntfy_machine.chars().any(char::is_control),
                "ntfy machine name must be at most 128 bytes without control characters"
            );
        }
        ensure!(
            (1..=3650).contains(&self.history_days),
            "History age must be 1–3650 days"
        );
        ensure!(
            (1..=4096).contains(&self.session_mib)
                && self.total_mib >= self.session_mib
                && self.total_mib <= 65536,
            "Invalid disk limits"
        );
        ensure!(
            (100..=20_000).contains(&self.scrollback_lines),
            "History lines must be 100–20000"
        );
        ensure!(
            (9.0..=32.0).contains(&self.font_size),
            "Font size must be 9–32"
        );
        ensure!(
            (1..=30).contains(&self.editor_close_timeout_secs),
            "Editor close timeout must be 1–30 seconds"
        );
        Ok(())
    }
}
