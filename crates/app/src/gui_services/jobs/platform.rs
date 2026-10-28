//! Platform jobs: clipboard, browser, and external-editor launches.
use super::super::submit::Services;
use crate::{Update, clipboard, external_editor, image_preview};
use anyhow::Result;
use std::path::PathBuf;
use terminator_core::async_service::CancellationToken;
use terminator_core::{Request, Response};

pub(crate) struct TestExternalInput {
    pub(in crate::gui_services) path: PathBuf,
    pub(in crate::gui_services) program: String,
    pub(in crate::gui_services) args: Vec<String>,
    pub(in crate::gui_services) cancel: CancellationToken,
}

pub(crate) struct LaunchExternalInput<'a> {
    pub(crate) path: PathBuf,
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    pub(crate) test: bool,
    pub(crate) _cancel: &'a CancellationToken,
}

impl Services {
    pub(in crate::gui_services) async fn paste_clipboard(
        &self,
        session: String,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        if let Some(text) = self.platform().run(&cancel, clipboard::read_paste).await? {
            updates.push(Update::ClipboardPaste(session, text));
        }
        Ok(())
    }

    pub(in crate::gui_services) async fn open_browser(
        &self,
        url: String,
        cancel: CancellationToken,
    ) -> Result<()> {
        self.platform()
            .run(&cancel, move || {
                open::that(url)?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub(in crate::gui_services) async fn open_external(
        &self,
        path: PathBuf,
        cancel: CancellationToken,
    ) -> Result<()> {
        let client = self.client();
        let (program, args) = if image_preview::supported(&path) {
            external_editor::image_opener()
        } else {
            let Response::State(state) = client.rpc(Request::Snapshot).await? else {
                anyhow::bail!("Expected editor settings snapshot")
            };
            (state.settings.external_editor, state.settings.external_args)
        };
        self.launch_external(LaunchExternalInput {
            path,
            program,
            args,
            test: false,
            _cancel: &cancel,
        })
        .await?;
        Ok(())
    }

    pub(in crate::gui_services) async fn test_external(
        &self,
        input: TestExternalInput,
    ) -> Result<()> {
        let TestExternalInput {
            path,
            program,
            args,
            cancel,
        } = input;
        self.launch_external(LaunchExternalInput {
            path,
            program,
            args,
            test: true,
            _cancel: &cancel,
        })
        .await?;
        Ok(())
    }

    async fn launch_external(&self, input: LaunchExternalInput<'_>) -> Result<()> {
        let LaunchExternalInput {
            path,
            program,
            args,
            test,
            _cancel,
        } = input;
        external_editor::launch_supervised(self.clone(), program, args, path).await?;
        if test {
            self.emit(Update::Info("External editor launched with draft settings. Check the selected file in the editor.".into())).await?;
        }
        Ok(())
    }
}
