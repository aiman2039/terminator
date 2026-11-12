use super::*;

/// A `JetBrains` tool window key: Cmd+digit on macOS, Alt+digit on Linux
/// (stored `command` means Ctrl+Shift there).
fn tool_window_key(digit: char) -> String {
    if cfg!(target_os = "macos") {
        format!("command+{digit}")
    } else {
        format!("alt+{digit}")
    }
}

/// Next/previous top-level tab: Cmd+Shift+] / [ on macOS (Safari,
/// `JetBrains`), Ctrl+PageDown / PageUp on Linux, where stored `command`
/// already means Ctrl+Shift and Ctrl+Shift+] is Next pane.
fn tab_step_key(forward: bool) -> String {
    match (cfg!(target_os = "macos"), forward) {
        (true, true) => "command+shift+]",
        (true, false) => "command+shift+[",
        (false, true) => "ctrl+PageDown",
        (false, false) => "ctrl+PageUp",
    }
    .into()
}

/// Previous/next shell prompt: Cmd+Up / Down on macOS (Terminal.app,
/// Ghostty), Ctrl+Shift+PageUp / PageDown on Linux (Ghostty), where
/// Ctrl+Shift+Up scrolls a line in most terminals.
fn prompt_jump_key(forward: bool) -> String {
    match (cfg!(target_os = "macos"), forward) {
        (true, true) => "command+Down",
        (true, false) => "command+Up",
        (false, true) => "ctrl+shift+PageDown",
        (false, false) => "ctrl+shift+PageUp",
    }
    .into()
}

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
            total_mib: 512,
            scrollback_lines: 10_000,
            font_size: 13.0,
            editor_close_timeout_secs: 1,
            diff_split_default: false,
            native_vim: true,
            inlay_hints: true,
            spell_check: true,
            keybindings: Settings::default_keybindings(),
            vnc_detached_timeout_minutes: 30,
            ssh_keepalive_seconds: crate::ssh::Tuning::DEFAULT.keepalive_seconds,
            ssh_reuse_minutes: crate::ssh::Tuning::DEFAULT.reuse_minutes,
            ssh_connect_timeout_seconds: crate::ssh::Tuning::DEFAULT.connect_timeout_seconds,
            ssh_command_timeout_seconds: crate::ssh::Tuning::DEFAULT.command_timeout_seconds,
            local_history_days: 7,
            local_history_mib: crate::local_history::DEFAULT_MIB,
            local_history_file_entries: crate::local_history::DEFAULT_FILE_ENTRIES,
            ntfy_options: NtfyOptions::default(),
        }
    }
}
impl Settings {
    /// Every chord is unique, except on Linux (where stored `command`
    /// already means Ctrl+Shift): Find in terminal and Find in Files share
    /// Ctrl+Shift+F, and the GUI picks one by keyboard focus.
    fn default_keybindings() -> std::collections::BTreeMap<String, String> {
        [
            ("new_terminal".into(), "command+T".into()),
            ("open_file".into(), "command+O".into()),
            ("split_up".into(), "command+alt+Up".into()),
            ("split_down".into(), "command+alt+D".into()),
            ("split_left".into(), "command+shift+Left".into()),
            ("split_right".into(), "command+shift+D".into()),
            ("next_tab".into(), tab_step_key(true)),
            ("prev_tab".into(), tab_step_key(false)),
            ("next_pane".into(), "command+]".into()),
            ("select_all".into(), "command+A".into()),
            ("find_in_terminal".into(), "command+F".into()),
            // JetBrains Find in Path; Find in terminal's chord on Linux.
            ("find_in_files".into(), "command+shift+F".into()),
            ("search_scrollback".into(), "command+alt+F".into()),
            ("copy_working_directory".into(), "command+alt+C".into()),
            // Was command+shift+R (see the GUI keymap migration).
            ("rename_terminal".into(), "command+alt+R".into()),
            // JetBrains Replace in Path.
            ("replace_in_files".into(), "command+shift+R".into()),
            ("close_session".into(), "command+W".into()),
            ("clear_scrollback".into(), "command+shift+K".into()),
            ("editor_save".into(), "command+S".into()),
            ("compare_disk".into(), "command+shift+Y".into()),
            ("toggle_left_sidebar".into(), "command+B".into()),
            ("toggle_right_sidebar".into(), "command+L".into()),
            ("toggle_ide_mode".into(), "command+alt+E".into()),
            ("move_to_main".into(), "command+alt+M".into()),
            ("move_to_strip".into(), "command+alt+Down".into()),
            // iTerm2's Maximize Active Pane; Ctrl+Shift+Enter on Linux.
            ("zoom_pane".into(), "command+shift+Enter".into()),
            ("open_settings".into(), "command+,".into()),
            ("open_palette".into(), "command+P".into()),
            ("goto_symbol".into(), "command+alt+O".into()),
            ("file_structure".into(), "command+F12".into()),
            ("navigate_back".into(), "command+alt+Left".into()),
            ("navigate_forward".into(), "command+alt+Right".into()),
            ("recent_files".into(), "command+E".into()),
            ("next_attention".into(), "command+shift+J".into()),
            ("ask_agent".into(), "command+alt+A".into()),
            ("new_browser".into(), "command+alt+B".into()),
            ("new_vnc".into(), "command+alt+V".into()),
            ("diff_next_change".into(), "F7".into()),
            ("diff_prev_change".into(), "shift+F7".into()),
            ("diff_next_file".into(), "alt+F7".into()),
            ("diff_prev_file".into(), "alt+shift+F7".into()),
            ("tab_switcher_back".into(), "ctrl+shift+Tab".into()),
            ("tab_switcher".into(), "ctrl+Tab".into()),
            ("tool_explorer".into(), tool_window_key('1')),
            ("tool_problems".into(), tool_window_key('6')),
            ("tool_structure".into(), tool_window_key('7')),
            ("tool_git".into(), tool_window_key('9')),
            ("tool_terminal".into(), "alt+F12".into()),
            ("select_in".into(), "alt+F1".into()),
            ("reopen_browser_tab".into(), "command+alt+T".into()),
            // Free in both keymaps; New terminal is command+T.
            ("new_agent_session".into(), "command+alt+N".into()),
            ("new_drawing".into(), "command+alt+X".into()),
            ("prev_prompt".into(), prompt_jump_key(false)),
            ("next_prompt".into(), prompt_jump_key(true)),
        ]
        .into()
    }

    pub fn ssh_tuning(&self) -> crate::ssh::Tuning {
        crate::ssh::Tuning {
            keepalive_seconds: self.ssh_keepalive_seconds,
            reuse_minutes: self.ssh_reuse_minutes,
            connect_timeout_seconds: self.ssh_connect_timeout_seconds,
            command_timeout_seconds: self.ssh_command_timeout_seconds,
        }
    }

    pub fn validate(&self) -> Result<(), SettingsError> {
        // Disabled ntfy accepts any channel/machine contents (including
        // leftovers) so turning the toggle off always saves. Enabling
        // requires a usable channel again, and delivery still re-validates.
        if self.ntfy_enabled {
            self.validate_ntfy()?;
        }
        if !(1..=3650).contains(&self.history_days) {
            return Err(SettingsError::HistoryDays {
                days: self.history_days,
            });
        }
        if !((1..=4096).contains(&self.session_mib)
            && self.total_mib >= self.session_mib
            && self.total_mib <= 65536)
        {
            return Err(SettingsError::DiskLimits {
                session_mib: self.session_mib,
                total_mib: self.total_mib,
            });
        }
        if !(100..=20_000).contains(&self.scrollback_lines) {
            return Err(SettingsError::ScrollbackLines {
                lines: self.scrollback_lines,
            });
        }
        if !(9.0..=32.0).contains(&self.font_size) {
            return Err(SettingsError::FontSize {
                size: self.font_size,
            });
        }
        if !(1..=30).contains(&self.editor_close_timeout_secs) {
            return Err(SettingsError::EditorCloseTimeout {
                seconds: self.editor_close_timeout_secs,
            });
        }
        if self.vnc_detached_timeout_minutes > 1440 {
            return Err(SettingsError::VncDetachedTimeout {
                minutes: self.vnc_detached_timeout_minutes,
            });
        }
        if self.local_history_days > 3650 {
            return Err(SettingsError::LocalHistoryDays {
                days: self.local_history_days,
            });
        }
        if !((1..=65536).contains(&self.local_history_mib)
            && (1..=10_000).contains(&self.local_history_file_entries))
        {
            return Err(SettingsError::LocalHistoryLimits {
                mib: self.local_history_mib,
                file_entries: self.local_history_file_entries,
            });
        }
        if !self.ssh_tuning().valid() {
            return Err(SettingsError::SshTuning {
                tuning: self.ssh_tuning(),
            });
        }
        Ok(())
    }

    fn validate_ntfy(&self) -> Result<(), SettingsError> {
        let valid_channel = (1..=128).contains(&self.ntfy_channel.len())
            && self
                .ntfy_channel
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if !valid_channel {
            return Err(SettingsError::NtfyChannel {
                channel: self.ntfy_channel.clone(),
            });
        }
        if self.ntfy_machine.len() > 128 || self.ntfy_machine.chars().any(char::is_control) {
            return Err(SettingsError::NtfyMachine {
                machine: self.ntfy_machine.clone(),
            });
        }
        let options = &self.ntfy_options;
        for url in [&options.server, &options.click] {
            if !(url.is_empty() || web_url(url)) {
                return Err(SettingsError::NtfyUrl { url: url.clone() });
            }
        }
        Ok(())
    }

    /// The ntfy server pushes go to: the configured one or `ntfy.sh`.
    #[must_use]
    pub fn ntfy_endpoint(&self) -> String {
        match self.ntfy_options.server.trim_end_matches('/') {
            "" => NTFY_DEFAULT_SERVER.into(),
            server => server.into(),
        }
    }
}

/// The daemon serves `Request::NtfyToken` and honors [`NtfyOptions`].
pub const NTFY_SERVER_CAPABILITY: &str = "ntfy-server-v1";
const NTFY_DEFAULT_SERVER: &str = "https://ntfy.sh";

/// ntfy push options (`ntfy-server-v1`). The access token is not here: the
/// daemon keeps it in the OS credential store.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NtfyOptions {
    /// A self-hosted server URL; empty means `https://ntfy.sh`.
    pub server: String,
    /// Opened when the push is tapped; empty sends no click action.
    pub click: String,
    /// Adds the hook's notice text (for example the question) to the push.
    pub include_text: bool,
}

/// An http(s) URL with a host, at most 1024 bytes, with no spaces or
/// control characters (it goes into a curl config line).
fn web_url(text: &str) -> bool {
    text.len() <= 1024
        && !text
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '"')
        && url::Url::parse(text)
            .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host().is_some())
}
