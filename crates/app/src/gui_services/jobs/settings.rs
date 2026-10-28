//! Settings, appearance, hook, and integration-install jobs.
use super::super::submit::Services;
use crate::preferences::UiPreferences;
use crate::{Update, notify_test};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use terminator_core::appearance::{AppearanceConfig, AppearanceFile, config_path};
use terminator_core::async_service::CancellationToken;
use terminator_core::{Request, Response};

pub(crate) struct SaveAppearanceInput<'a> {
    pub(in crate::gui_services) theme: Box<AppearanceConfig>,
    pub(in crate::gui_services) expected: String,
    pub(in crate::gui_services) cancel: CancellationToken,
    pub(in crate::gui_services) updates: &'a mut Vec<Update>,
}

pub(crate) struct InstallInput<'a> {
    pub(in crate::gui_services) kind: String,
    pub(in crate::gui_services) remove: bool,
    pub(in crate::gui_services) cancel: CancellationToken,
    pub(in crate::gui_services) updates: &'a mut Vec<Update>,
}

pub(crate) struct TestNtfyInput<'a> {
    pub(in crate::gui_services) channel: String,
    pub(in crate::gui_services) machine: String,
    pub(in crate::gui_services) cancel: CancellationToken,
    pub(in crate::gui_services) updates: &'a mut Vec<Update>,
}

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
                anyhow::Ok(prefs)
            })
            .await
            .map_err(|e| format!("Save UI preferences: {e:#}"));
        updates.push(Update::PreferencesSaved(Box::new(result)));
        Ok(())
    }

    pub(in crate::gui_services) async fn save_appearance(
        &self,
        input: SaveAppearanceInput<'_>,
    ) -> Result<()> {
        let SaveAppearanceInput {
            theme,
            expected,
            cancel,
            updates,
        } = input;
        let client = self.client();
        let paths = client.paths.clone();
        let file = client
            .catalog
            .run(&cancel, move || {
                Ok::<_, anyhow::Error>(AppearanceFile::save(
                    &config_path(&paths)?,
                    &theme,
                    &expected,
                )?)
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
                anyhow::Ok(
                    terminator_integrations::AGENTS
                        .iter()
                        .map(|kind| {
                            (
                                (*kind).to_string(),
                                terminator_integrations::installed(&home, kind),
                            )
                        })
                        .collect(),
                )
            })
            .await?;
        updates.push(Update::HookStatus(status));
        Ok(())
    }

    pub(in crate::gui_services) async fn install(&self, input: InstallInput<'_>) -> Result<()> {
        let InstallInput {
            kind,
            remove,
            cancel,
            updates,
        } = input;
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
                let path =
                    terminator_integrations::install_at(terminator_integrations::InstallAtInput {
                        home: Path::new(&home),
                        kind: &kind,
                        helper: &helper,
                        remove,
                        paths: &paths,
                    })?;
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

    pub(in crate::gui_services) async fn test_ntfy(&self, input: TestNtfyInput<'_>) -> Result<()> {
        let TestNtfyInput {
            channel,
            machine,
            cancel,
            updates,
        } = input;
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
