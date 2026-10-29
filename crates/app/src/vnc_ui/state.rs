use super::*;
pub(crate) struct VncViewState {
    pub(crate) texture: Option<TextureHandle>,
    pub(crate) cursor_texture: Option<TextureHandle>,
    pub(crate) cursor_signature: Option<String>,
    pub(crate) cursor_hotspot: [u16; 2],
    pub(crate) monitors: Vec<terminator_core::vnc::VncMonitor>,
    pub(crate) selected_monitor: Option<u32>,
    pub(crate) resize_supported: bool,
    pub(crate) resize_target: Option<(u16, u16)>,
    pub(crate) resize_changed: Option<Instant>,
    pub(crate) resize_sent: Option<(u16, u16)>,
    pub(crate) paste_as_keystrokes: bool,
    pub(crate) capture_shortcuts: bool,
    pub(crate) focus_id: Option<egui::Id>,
    pub(crate) visible_last_frame: bool,
    pub(crate) options_initialized: bool,
    pub(crate) draft_options: terminator_core::vnc::VncOptions,
    pub(crate) transfer_remote: String,
    pub(crate) transfer_status: Option<terminator_core::vnc::VncTransferStatus>,
    pub(crate) transfer_poll: Option<Instant>,
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) revision: u64,
    pub(crate) status: VncSessionStatus,
    pub(crate) error: Option<String>,
    /// The last received clipboard text. Frames repeat it until it changes.
    pub(crate) remote_clipboard: Option<String>,
    pub(crate) fetching: bool,
    pub(crate) last_poll: Option<Instant>,
    /// Newest cursor image received (`vnc-shm-v1` sends it only on change).
    pub(crate) cursor_serial: u64,
    /// The shared-memory frame file was unusable: stay on compressed frames.
    pub(crate) shm_failed: bool,
    pub(crate) scale: super::scale::VncScale,
    /// Shortcut capture before fullscreen turned it on; restored on exit.
    pub(crate) capture_before_fullscreen: Option<bool>,
    /// The toolbar's Ctrl+Alt+Del waits for confirmation.
    pub(crate) confirm_ctrl_alt_del: bool,
    /// Remote folder for files dropped on the screen (absolute path).
    pub(crate) upload_dir: String,
    pub(crate) last_pointer: Option<(u16, u16, u8)>,
    /// Ask the next frame request for the whole screen: set when the
    /// texture cannot take an incremental update (missing, wrong size, or
    /// a decode that produced nothing).
    pub(crate) force_full: bool,
    /// Keyboard focus on the remote screen last frame: a fall from true to
    /// false releases every held key and modifier.
    pub(crate) focused: bool,
    /// Vergen modifier state (shift/ctrl/alt/meta) held for the remote:
    /// the viewer presses and releases the remote modifier keys itself so
    /// Shift+click and Ctrl+C land as the remote expects.
    pub(crate) modifier_bits: u8,
    /// Keysyms currently held down for the remote, released when this
    /// view loses focus so nothing sticks.
    pub(crate) held_keys: Vec<u32>,
    pub(crate) draft_name: String,
    pub(crate) draft_host: String,
    pub(crate) draft_port: String,
    pub(crate) draft_display: String,
    pub(crate) draft_username: String,
    pub(crate) draft_password: String,
    pub(crate) draft_show_password: bool,
    /// Save the password typed into the prompt to the keychain.
    pub(crate) draft_remember: bool,
    pub(crate) draft_shared: bool,
    pub(crate) draft_view_only: bool,
    /// False when the draft came from a saved host: the pane then only
    /// asks for that host's password.
    pub(crate) save_profile: bool,
    /// Quick-connect text in the address book: `host:display` or `host::port`.
    pub(crate) address: String,
    pub(crate) speed: super::speed::ViewSpeed,
    /// Session whose "password required" failure already opened the
    /// password prompt, so going back to all hosts is not undone.
    pub(crate) prompted_for: Option<String>,
}

impl Default for VncViewState {
    fn default() -> Self {
        Self {
            texture: None,
            cursor_texture: None,
            cursor_signature: None,
            cursor_hotspot: [0, 0],
            monitors: Vec::new(),
            selected_monitor: None,
            resize_supported: false,
            resize_target: None,
            resize_changed: None,
            resize_sent: None,
            paste_as_keystrokes: false,
            capture_shortcuts: false,
            focus_id: None,
            visible_last_frame: false,
            options_initialized: false,
            draft_options: terminator_core::vnc::VncOptions::default(),
            transfer_remote: String::new(),
            transfer_status: None,
            transfer_poll: None,
            width: 0,
            height: 0,
            revision: 0,
            status: VncSessionStatus::Disconnected,
            error: None,
            fetching: false,
            remote_clipboard: None,
            last_poll: None,
            cursor_serial: 0,
            shm_failed: false,
            scale: super::scale::VncScale::Fit,
            capture_before_fullscreen: None,
            confirm_ctrl_alt_del: false,
            upload_dir: String::new(),
            last_pointer: None,
            force_full: false,
            focused: false,
            modifier_bits: 0,
            held_keys: Vec::new(),
            draft_name: String::new(),
            draft_host: "127.0.0.1".into(),
            draft_port: DEFAULT_CONNECTION_PORT.to_string(),
            draft_display: display_for_port(&DEFAULT_CONNECTION_PORT.to_string()),
            draft_username: String::new(),
            draft_password: String::new(),
            draft_show_password: false,
            draft_remember: true,
            draft_shared: true,
            draft_view_only: false,
            save_profile: true,
            address: String::new(),
            speed: super::speed::ViewSpeed::default(),
            prompted_for: None,
        }
    }
}

pub(crate) struct VncSidebarState {
    pub(crate) options: terminator_core::vnc::VncOptions,
    pub(crate) filter: String,
    pub(crate) error: Option<String>,
    pub(crate) editing_id: Option<String>,
    pub(crate) form_open: bool,
    pub(crate) name: String,
    pub(crate) host: String,
    pub(crate) port: String,
    pub(crate) display: String,
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) show_password: bool,
    pub(crate) save_password: bool,
    pub(crate) shared: bool,
    pub(crate) view_only: bool,
    /// Saved host awaiting removal confirmation.
    pub(crate) confirm_remove: Option<String>,
    pub(crate) tab: PropertiesTab,
    /// The edited host has a password in the OS keychain.
    pub(crate) credential_saved: bool,
    /// Remove the saved password on Save.
    pub(crate) forget_password: bool,
    /// Shown under the form after `~/.ssh/config` filled the SSH settings.
    pub(crate) ssh_notice: Option<String>,
    pub(crate) ssh_probe: super::ssh_probe::SshProbe,
    /// Address-book card selected for keyboard actions.
    pub(crate) selected_host: Option<String>,
    pub(crate) thumbnails: super::thumbnails::ThumbnailCache,
    /// Labels of the edited host, comma-separated.
    pub(crate) labels: String,
    /// Address book shows only hosts with this label.
    pub(crate) label_filter: Option<String>,
    /// Address book shows rows instead of cards.
    pub(crate) list_view: bool,
    /// Open "Import hosts" panel.
    pub(crate) import: Option<super::import::ImportState>,
}

/// Pages of the host Properties editor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum PropertiesTab {
    #[default]
    General,
    Security,
    Options,
}

impl Default for VncSidebarState {
    fn default() -> Self {
        // The same starting point as [`Self::begin_new`]: a form that opens
        // without an explicit reset still shows a usable host and port.
        Self {
            options: terminator_core::vnc::VncOptions::default(),
            filter: String::new(),
            error: None,
            editing_id: None,
            form_open: false,
            name: String::new(),
            host: "127.0.0.1".into(),
            port: DEFAULT_CONNECTION_PORT.to_string(),
            display: display_for_port(&DEFAULT_CONNECTION_PORT.to_string()),
            username: String::new(),
            password: String::new(),
            show_password: false,
            save_password: true,
            shared: true,
            view_only: false,
            confirm_remove: None,
            tab: PropertiesTab::General,
            credential_saved: false,
            forget_password: false,
            ssh_notice: None,
            ssh_probe: super::ssh_probe::SshProbe::default(),
            selected_host: None,
            thumbnails: super::thumbnails::ThumbnailCache::default(),
            labels: String::new(),
            label_filter: None,
            list_view: false,
            import: None,
        }
    }
}

impl VncSidebarState {
    pub(crate) fn begin_new(&mut self) {
        self.options = terminator_core::vnc::VncOptions::default();
        self.error = None;
        self.editing_id = None;
        self.form_open = true;
        self.name.clear();
        self.host = "127.0.0.1".into();
        self.port = DEFAULT_CONNECTION_PORT.to_string();
        self.display = display_for_port(&self.port);
        self.username.clear();
        self.password.clear();
        self.show_password = false;
        self.save_password = true;
        self.shared = true;
        self.view_only = false;
        self.labels.clear();
        self.reset_editor_extras(false);
    }

    fn reset_editor_extras(&mut self, credential_saved: bool) {
        self.tab = PropertiesTab::General;
        self.credential_saved = credential_saved;
        self.forget_password = false;
        self.ssh_notice = None;
        self.ssh_probe = super::ssh_probe::SshProbe::default();
    }

    pub(crate) fn begin_edit(&mut self, profile: &VncProfile) {
        self.error = None;
        self.options = profile.options.clone();
        self.editing_id = Some(profile.id.clone());
        self.form_open = true;
        self.name.clone_from(&profile.name);
        self.host.clone_from(&profile.host);
        self.port = profile.port.to_string();
        self.display = display_for_port(&self.port);
        self.username.clone_from(&profile.username);
        self.password.clone_from(&profile.password);
        self.show_password = false;
        self.save_password = profile.credential_saved || !profile.password.is_empty();
        self.shared = profile.shared;
        self.view_only = profile.view_only;
        self.labels = profile.labels.join(", ");
        self.reset_editor_extras(profile.credential_saved);
    }
}
