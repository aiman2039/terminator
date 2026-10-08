//! Settings center pane and appearance preview.
mod body;
mod footer;
mod nav;
#[cfg(test)]
mod settings_tests;
mod state;

pub(crate) use state::{BrowseTarget, SettingsPending, SettingsSection};
