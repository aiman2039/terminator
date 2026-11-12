//! Settings center pane and appearance preview.
mod body;
mod footer;
mod nav;
mod rebase;
mod sections;
#[cfg(test)]
mod settings_tests;
mod state;

pub(crate) use body::ThemeImport;
pub(crate) use state::{BrowseTarget, Probes, SettingsPending, SettingsSection};
