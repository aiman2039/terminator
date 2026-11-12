//! The system-wide hotkey through the `global-hotkey` crate: Carbon
//! `RegisterEventHotKey` on macOS, `XGrabKey` on X11, `RegisterHotKey` on
//! Windows. Wayland has no global key grab; its GlobalShortcuts portal
//! needs a D-Bus portal crate this workspace does not have, so the hotkey
//! stays off there and Settings says so.
use eframe::egui;
use global_hotkey::hotkey::HotKey;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Registration state of the quick terminal hotkey.
#[derive(Default)]
pub(crate) struct Hotkey {
    manager: Option<global_hotkey::GlobalHotKeyManager>,
    /// The binding text and hotkey now registered.
    registered: Option<(String, HotKey)>,
    /// The binding text last tried, registered or not.
    tried: Option<String>,
    /// Why the hotkey is not active, for Settings.
    pub(crate) note: Option<String>,
    pressed: Arc<AtomicBool>,
    handler: bool,
}

/// True in a Wayland session, where no global key grab exists.
pub(crate) fn wayland_session() -> bool {
    cfg!(target_os = "linux")
        && (std::env::var("XDG_SESSION_TYPE")
            .is_ok_and(|kind| kind.eq_ignore_ascii_case("wayland"))
            || (std::env::var_os("WAYLAND_DISPLAY").is_some()
                && std::env::var_os("DISPLAY").is_none()))
}

/// The global hotkey for a Settings binding such as `command+shift+T`.
pub(crate) fn to_hotkey(binding: &str) -> Option<HotKey> {
    let chord = crate::shortcuts::parse(binding)?;
    let modifiers = chord.modifiers;
    let mut parts = Vec::new();
    if cfg!(target_os = "macos") {
        if modifiers.mac_cmd || (modifiers.command && !modifiers.ctrl) {
            parts.push("super");
        }
        if modifiers.ctrl {
            parts.push("control");
        }
    } else if modifiers.ctrl || modifiers.command {
        parts.push("control");
    }
    if modifiers.alt {
        parts.push("alt");
    }
    if modifiers.shift {
        parts.push("shift");
    }
    if parts.is_empty() {
        // A bare key would take that key from every application.
        return None;
    }
    [chord.key.name(), chord.key.symbol_or_name()]
        .into_iter()
        .find_map(|key| format!("{}+{key}", parts.join("+")).parse().ok())
}

impl Hotkey {
    /// Register `binding` (empty: none) when it changed. Must run on the
    /// main thread (macOS). Errors leave the hotkey off with a note.
    pub(crate) fn sync(&mut self, ctx: &egui::Context, binding: &str) {
        if self.tried.as_deref() == Some(binding) {
            return;
        }
        self.tried = Some(binding.to_owned());
        self.unregister();
        self.note = None;
        if binding.trim().is_empty() {
            return;
        }
        if wayland_session() {
            self.note = Some("The quick terminal hotkey is not available on Wayland.".into());
            return;
        }
        let Some(hotkey) = to_hotkey(binding) else {
            self.note = Some("This key cannot be a system-wide hotkey; add a modifier.".into());
            return;
        };
        if let Err(error) = self.register(ctx, hotkey) {
            self.note = Some(format!("The system did not accept the hotkey: {error}"));
            return;
        }
        self.registered = Some((binding.to_owned(), hotkey));
    }

    fn register(&mut self, ctx: &egui::Context, hotkey: HotKey) -> Result<(), String> {
        if self.manager.is_none() {
            self.manager =
                Some(global_hotkey::GlobalHotKeyManager::new().map_err(|e| e.to_string())?);
        }
        if !self.handler {
            let pressed = Arc::clone(&self.pressed);
            let ctx = ctx.clone();
            global_hotkey::GlobalHotKeyEvent::set_event_handler(Some(
                move |event: global_hotkey::GlobalHotKeyEvent| {
                    if event.state() == global_hotkey::HotKeyState::Pressed {
                        pressed.store(true, Ordering::Release);
                        ctx.request_repaint();
                    }
                },
            ));
            self.handler = true;
        }
        self.manager
            .as_ref()
            .ok_or_else(|| "no hotkey manager".to_owned())?
            .register(hotkey)
            .map_err(|e| e.to_string())
    }

    fn unregister(&mut self) {
        if let (Some(manager), Some((_, hotkey))) = (&self.manager, self.registered.take()) {
            let _ = manager.unregister(hotkey);
        }
    }

    /// The hotkey is registered with the system.
    pub(crate) fn active(&self) -> bool {
        self.registered.is_some()
    }

    /// True once per press since the last call.
    pub(crate) fn take_press(&self) -> bool {
        self.pressed.swap(false, Ordering::AcqRel)
    }
}
