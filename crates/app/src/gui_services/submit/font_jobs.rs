//! System font scan and font file reads on the filesystem pool.
use super::Services;
use crate::Update;
use crate::appearance::fonts::{self, Op, Reply};
use anyhow::Result;
use terminator_core::async_service::CancellationToken;

impl Services {
    /// Always answers, so Settings never waits on a lost reply.
    pub(super) async fn font_job(
        &self,
        op: Op,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let failed = op.clone();
        let reply = self
            .fs()
            .run(&cancel, move || anyhow::Ok(fonts::perform(op)))
            .await
            .unwrap_or_else(|error| {
                let error = format!("System fonts: {error:#}");
                match failed {
                    Op::List => Reply::Listed(Err(error)),
                    Op::Load(prefs) => Reply::Loaded(prefs, Err(error)),
                }
            });
        updates.push(Update::Fonts(reply));
        Ok(())
    }
}
