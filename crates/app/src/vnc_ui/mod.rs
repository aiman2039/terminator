mod address_book;
mod book_keys;
mod card;
mod clipboard;
mod connect;
mod display;
pub(crate) use display::VncScreenInput;
mod endpoint;
mod error_fix;
mod form;
mod frame;
mod keyboard;
mod live;
mod page;
mod profiles;
mod quit;
#[cfg(test)]
pub(crate) use quit::QuitChoice;
mod scale;
mod settings;
mod shm;
mod sidebar;
mod speed;
mod ssh_probe;
mod state;
mod tabs;
mod thumbnails;
mod toolbar;
mod transfer;
mod view;
use clipboard::remote_clipboard_events;
use connect::{StartVncConnect, VncConnectPage, VncConnectTab};
use display::vnc_screen;
#[cfg(test)]
use endpoint::port_for_display;
use endpoint::{connection_endpoint, display_for_port, port_display_fields};
pub(crate) use frame::{VncDecodedFrame, decode_frame};
use keyboard::{
    MODIFIER_KEYSYMS, accelerator_keysym, egui_key_to_x11, modifier_bits, modifier_transitions,
};
pub(crate) use state::{PropertiesTab, VncSidebarState, VncViewState};
use std::{
    io::Read,
    time::{Duration, Instant},
};
use toolbar::VncToolbar;

use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
#[cfg(test)]
use eframe::egui::Color32;
use eframe::egui::{self, ColorImage, RichText, TextureHandle};
use flate2::read::ZlibDecoder;
use terminator_core::{
    Request, VNC_CAPABILITY, id,
    vnc::{VncFrame, VncInputEvent, VncProfile, VncRegion, VncSession, VncSessionStatus},
};

use crate::preferences::VncPasteShortcut;
use crate::{After, App, Job, Tab, appearance, icons};

// New forms start at display 66; saved profiles retain their explicit port.
const DEFAULT_CONNECTION_PORT: u16 = 5966;

struct PendingVncConnect {
    profile_id: Option<String>,
    host: String,
    port: u16,
    username: String,
    password: String,
    shared: bool,
    view_only: bool,
    label: Option<String>,
    save_profile: bool,
    options: terminator_core::vnc::VncOptions,
}

pub(crate) struct VncViewInput<'a> {
    pub(crate) tab_id: &'a str,
    pub(crate) session_id: &'a mut String,
    pub(crate) profile_id: &'a mut Option<String>,
    pub(crate) host: &'a mut String,
    pub(crate) port: &'a mut u16,
    pub(crate) label: &'a mut String,
}

impl App {
    pub(crate) fn vnc_supported(&self) -> bool {
        !self.state_loaded
            || self
                .state
                .capabilities
                .iter()
                .any(|cap| cap == VNC_CAPABILITY)
    }

    pub(crate) fn vnc_features_supported(&self) -> bool {
        self.state
            .capabilities
            .iter()
            .any(|cap| cap == terminator_core::vnc::FEATURES_CAPABILITY)
    }
    /// Why the running daemon cannot honor `options`, if it cannot. An
    /// older daemon drops fields it does not know, so the check runs here.
    pub(super) fn vnc_options_unsupported(
        &self,
        options: &terminator_core::vnc::VncOptions,
        features: bool,
    ) -> Option<&'static str> {
        if *options != terminator_core::vnc::VncOptions::default() && !features {
            return Some("Restart the session service to use advanced VNC options");
        }
        let start_command = self
            .state
            .capabilities
            .iter()
            .any(|cap| cap == terminator_core::vnc::START_COMMAND_CAPABILITY);
        (!options.ssh_start_command.is_empty() && !start_command)
            .then_some("Restart the session service to run a start command on the host")
    }
    fn session_features(&self, sid: &str) -> bool {
        self.session_capability(sid, terminator_core::vnc::FEATURES_CAPABILITY)
    }
    /// Whether the daemon generation that owns `sid` advertises `capability`.
    fn session_capability(&self, sid: &str, capability: &str) -> bool {
        let current = || self.state.capabilities.iter().any(|c| c == capability);
        let Some(session) = self.state.vnc_sessions.iter().find(|s| s.id == sid) else {
            return current();
        };
        if session.generation == self.state.generation {
            return current();
        }
        self.state.generations.iter().any(|g| {
            g.owner.id == session.generation && g.capabilities.iter().any(|c| c == capability)
        })
    }
    pub(crate) fn vnc_captures_shortcuts(&self, ctx: &egui::Context) -> bool {
        self.vnc_views.values().any(|view| {
            view.focused
                && view.capture_shortcuts
                && view.visible_last_frame
                && view
                    .focus_id
                    .is_some_and(|id| ctx.memory(|m| m.has_focus(id)))
        })
    }
    /// Leave VNC fullscreen and give each view back the shortcut capture
    /// it had before fullscreen turned it on.
    pub(crate) fn exit_vnc_fullscreen(&mut self, ctx: &egui::Context) {
        self.vnc_fullscreen = None;
        for view in self.vnc_views.values_mut() {
            if let Some(previous) = view.capture_before_fullscreen.take() {
                view.capture_shortcuts = previous;
            }
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
    }
}

#[derive(Default)]
pub(super) struct VncScreenOutcome {
    events: Vec<VncInputEvent>,
    activated: bool,
}

use egui::Widget;

#[cfg(test)]
mod tests;
