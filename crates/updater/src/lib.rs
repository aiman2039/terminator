//! Sparkle is loaded only from a configured app bundle. Development launches
//! never consult the production feed. `NSApplication`'s delegate remains winit's.
//! The application menu always has Check for Updates…; Sparkle is not required
//! for that item to exist. The macOS menu-bar status item lives in this crate
//! too, because the app crate forbids unsafe.
//!
//! `AppKit` method swizzling keeps `unsafe` in this crate. Other Terminator crates forbid it.
#[cfg(any(target_os = "macos", test))]
mod schedule;

#[cfg(any(target_os = "macos", test))]
struct FeedEligibility {
    isolated: bool,
    fixture: bool,
    debug: bool,
    bundled: bool,
    has_keys: bool,
}

#[cfg(any(target_os = "macos", test))]
fn skip_production_feed(input: FeedEligibility) -> bool {
    let FeedEligibility {
        isolated,
        fixture,
        debug,
        bundled,
        has_keys,
    } = input;
    !fixture && (isolated || debug || !bundled || !has_keys)
}

#[cfg(any(target_os = "macos", test))]
fn default_automatic_checks(sparkle_choice: Option<bool>) -> bool {
    sparkle_choice.unwrap_or(true)
}
#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
pub use macos::*;

/// One pending agent notice in the menu-bar status menu. The GUI rebuilds
/// this list every frame; the status item only rebuilds its `NSMenu` when the
/// serialized list changes, and reports the picked id back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusMenuItem {
    pub id: String,
    pub title: String,
}

#[cfg(target_os = "macos")]
mod status_item;
#[cfg(target_os = "macos")]
pub use status_item::{
    sync_status_item, sync_status_menu, take_status_click, take_status_selection,
};

#[cfg(not(target_os = "macos"))]
mod other {
    use eframe::egui;
    pub struct Updater;
    impl Updater {
        pub fn poll(&self) {}
        pub fn new(_: &egui::Context) -> Self {
            Self
        }
        #[cfg(feature = "test-support")]
        pub fn available(&self) -> bool {
            false
        }
        #[cfg(feature = "test-support")]
        pub fn menu_installed(&self) -> bool {
            false
        }
        pub fn settings(&self, ui: &mut egui::Ui) {
            ui.heading("Updates");
            ui.hyperlink_to(
                "Download releases",
                "https://github.com/aiman2039/terminator/releases",
            );
        }
    }
    #[cfg(feature = "test-support")]
    pub fn fixture_native_quit() {}
    pub fn termination_requested() -> bool {
        false
    }
    pub fn termination_cancelled() -> bool {
        false
    }
    pub fn cancel_termination() {}
    pub fn complete_termination(ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}
#[cfg(not(target_os = "macos"))]
pub use other::*;

#[cfg(not(target_os = "macos"))]
pub fn sync_status_item(_png: &[u8], _width_pt: f32, _height_pt: f32) {}

#[cfg(not(target_os = "macos"))]
pub fn take_status_click() -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub fn sync_status_menu(_items: &[StatusMenuItem]) {}

#[cfg(not(target_os = "macos"))]
pub fn take_status_selection() -> Option<String> {
    None
}

#[cfg(test)]
mod tests;
