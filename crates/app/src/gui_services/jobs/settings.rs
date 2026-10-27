//! Settings, appearance, hook, and integration-install jobs.
use super::super::submit::Services;
use crate::preferences::UiPreferences;
use crate::{Update, notify_test};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use terminator_core::appearance::{AppearanceConfig, AppearanceFile, config_path};
use terminator_core::async_service::CancellationToken;
use terminator_core::{Request, Response};

impl Services {
    pub(in crate::gui_services) async fn save_preferences(
        &self,
        prefs: UiPreferences,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let client = self.client();
        let data = client.paths.data.clone();
        let result = client
            .catalog
            .run(&cancel, move || {
                prefs.save(&data)?;
                Ok(prefs)
            })
            .await
            .map_err(|e| format!("Save UI preferences: {e:#}"));
        updates.push(Update::PreferencesSaved(Box::new(result)));
        Ok(())
    }

    pub(in crate::gui_services) async fn save_appearance(
        &self,
        theme: Box<AppearanceConfig>,
        expected: String,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let client = self.client();
        let paths = client.paths.clone();
        let file = client
            .catalog
            .run(&cancel, move || {
                AppearanceFile::save(&config_path(&paths)?, &theme, &expected)
            })
            .await?;
        updates.push(Update::Appearance(Box::new(file)));
        Ok(())
    }

    pub(in crate::gui_services) async fn migrate_settings(
        &self,
        attention: bool,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let client = self.client();
        let Response::State(mut state) = client.rpc(Request::Snapshot).await? else {
            anyhow::bail!("Expected settings snapshot")
        };
        if attention {
            state.settings.notifications_side = true;
            state
                .settings
                .keybindings
                .entry("open_file".into())
                .or_insert_with(|| "command+O".into());
        } else {
            state.settings.font_size = 13.0;
        }
        anyhow::ensure!(
            matches!(
                client.rpc(Request::Settings(state.settings)).await?,
                Response::Ok
            ),
            "Settings not acknowledged"
        );
        updates.push(if attention {
            Update::AttentionMigrated(Ok(()))
        } else {
            Update::TypographyMigrated
        });
        if let Response::State(state) = client.rpc(Request::Snapshot).await? {
            updates.push(Update::State(state));
        }
        Ok(())
    }

    pub(in crate::gui_services) async fn hook_status(
        &self,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let client = self.client();
        let status = client
            .catalog
            .run(&cancel, move || {
                let home = std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_default();
                Ok(terminator_integrations::AGENTS
                    .iter()
                    .map(|kind| {
                        (
                            (*kind).to_string(),
                            terminator_integrations::installed(&home, kind),
                        )
                    })
                    .collect())
            })
            .await?;
        updates.push(Update::HookStatus(status));
        Ok(())
    }

    pub(in crate::gui_services) async fn install(
        &self,
        kind: String,
        remove: bool,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let client = self.client();
        let paths = client.paths.clone();
        let message = client
            .catalog
            .run(&cancel, move || {
                anyhow::ensure!(
                    remove || terminator_core::agents::find_installed(&kind).is_some(),
                    "Install the {kind} CLI before configuring its hooks"
                );
                let home = std::env::var_os("HOME").context("No home directory")?;
                let helper =
                    terminator_core::sibling_exe(&std::env::current_exe()?, "terminator-hook");
                let path = terminator_integrations::install_at(
                    Path::new(&home),
                    &kind,
                    &helper,
                    remove,
                    &paths,
                )?;
                Ok(format!(
                    "{} hooks: {}",
                    if remove { "Removed" } else { "Installed" },
                    path.display()
                ))
            })
            .await?;
        updates.push(Update::Info(message));
        Ok(())
    }

    pub(in crate::gui_services) async fn test_ntfy(
        &self,
        channel: String,
        machine: String,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let delivered = self
            .platform()
            .run(&cancel, move || {
                notify_test::send(notify_test::ENDPOINT, &channel, &machine)
            })
            .await;
        match delivered {
            Ok(()) => updates.push(Update::Info(
                "ntfy test posted. If your phone stayed silent, open the channel in ntfy to subscribe."
                    .into(),
            )),
            Err(error) => updates.push(Update::Error(format!(
                "ntfy test failed: {error:#}"
            ))),
        }
        Ok(())
    }
}
