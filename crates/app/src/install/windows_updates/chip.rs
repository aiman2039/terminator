//! Status-bar "Update vX" chip. Sparkle owns macOS updates and does not tell
//! the GUI about an available release, so the chip is Windows and Linux only.

use super::chip_version;
use crate::*;

/// True when a Linux package manager installed this GUI (`.deb`, `.rpm`,
/// AUR): it updates only through that package manager.
fn package_managed() -> bool {
    cfg!(target_os = "linux")
        && std::env::current_exe()
            .is_ok_and(|exe| terminator_core::distribution::package_managed(&exe))
}

/// The chip tooltip. A package-manager install says how to update.
pub(crate) fn chip_hint(version: &str, package_managed: bool) -> String {
    if package_managed {
        format!(
            "Terminator {version} is available. This copy was installed by a package \
             manager: update it with apt, dnf or your AUR helper. Click to open \
             Settings → Updates."
        )
    } else {
        format!("Terminator {version} is available. Click to open Settings → Updates.")
    }
}

impl App {
    pub(crate) fn update_status_chip(&mut self, ui: &mut egui::Ui) {
        let Some(version) = chip_version(
            self.windows_updates.available_version(),
            self.preferences.skipped_update_version.as_deref(),
        )
        .map(str::to_owned) else {
            return;
        };
        let label = format!("Update {version}");
        let response = appearance::icon_label_button(appearance::IconLabelButtonInput {
            ui,
            icon: "Download",
            label: &label,
            height: appearance::TOOLBAR_BUTTON,
        })
        .on_hover_text(chip_hint(&version, package_managed()));
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "status-update-chip", response.rect);
        if response.clicked() {
            self.open_installation_settings();
        }
    }
}
