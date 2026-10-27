//! Platform jobs: clipboard, browser, and external-editor launches.
use super::super::submit::Services;
use crate::{Update, clipboard, external_editor, image_preview};
use anyhow::Result;
use std::path::PathBuf;
use terminator_core::async_service::CancellationToken;
use terminator_core::{Request, Response};

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
        self.launch_external(path, program, args, false, &cancel)
            .await?;
        Ok(())
    }

    pub(in crate::gui_services) async fn test_external(
        &self,
        path: PathBuf,
        program: String,
        args: Vec<String>,
        cancel: CancellationToken,
    ) -> Result<()> {
        self.launch_external(path, program, args, true, &cancel)
            .await?;
        Ok(())
    }

    async fn launch_external(
        &self,
        path: PathBuf,
        program: String,
        args: Vec<String>,
        test: bool,
        _cancel: &CancellationToken,
    ) -> Result<()> {
        external_editor::launch_supervised(self.clone(), program, args, path).await?;
        if test {
            self.emit(Update::Info("External editor launched with draft settings. Check the selected file in the editor.".into())).await?;
        }
        Ok(())
    }
}
