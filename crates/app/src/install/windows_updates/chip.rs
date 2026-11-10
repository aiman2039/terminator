//! Status-bar "Update vX" chip. Sparkle owns macOS updates and does not tell
//! the GUI about an available release, so the chip is Windows and Linux only.

use super::chip_version;
use crate::*;

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
        .on_hover_text(format!(
            "Terminator {version} is available. Click to open Settings → Updates."
        ));
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "status-update-chip", response.rect);
        if response.clicked() {
            self.open_installation_settings();
        }
    }
}
