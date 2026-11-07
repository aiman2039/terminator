//! Settings center pane and appearance preview.
mod body;
mod footer;
mod nav;
mod rebase;
#[cfg(test)]
mod settings_tests;
mod state;

pub(crate) use state::{BrowseTarget, Probes, SettingsPending, SettingsSection};
