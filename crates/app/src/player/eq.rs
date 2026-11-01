use super::controller::Controller;
#[cfg(all(test, unix))]
use super::engine;
use super::engine::EqSettings;
#[cfg(all(test, unix))]
use anyhow::Result;
use std::time::Duration;
#[derive(Clone, Copy)]
pub(super) struct EqPreset {
    pub(super) preamp: f32,
    pub(super) bands: [f32; 10],
}

pub(super) fn eq_flat() -> EqPreset {
    EqPreset {
        preamp: 0.5,
        bands: [0.5; 10],
    }
}

pub(super) fn eq_bass() -> EqPreset {
    EqPreset {
        preamp: 0.55,
        bands: [0.82, 0.76, 0.68, 0.58, 0.52, 0.5, 0.48, 0.5, 0.5, 0.5],
    }
}

pub(super) fn eq_treble() -> EqPreset {
    EqPreset {
        preamp: 0.5,
        bands: [0.42, 0.45, 0.48, 0.5, 0.52, 0.58, 0.66, 0.74, 0.8, 0.84],
    }
}

impl Controller {
    /// Push equalizer edits to the audio engine; no-op when unchanged.
    pub(super) fn sync_eq(&mut self) {
        let settings = EqSettings {
            on: self.eq_on,
            preamp: self.eq_preamp,
            bands: self.eq_bands,
        };
        if self.eq_sent != Some(settings) {
            self.engine.equalizer(settings);
            self.eq_sent = Some(settings);
        }
    }
}

pub(super) fn format_clock(duration: Duration) -> String {
    let total = duration.as_secs();
    format!("{}:{:02}", total / 60, total % 60)
}

#[cfg(test)]
#[cfg(unix)]
pub(crate) async fn fixture_download(url: String) -> Result<()> {
    let client = reqwest::Client::builder().no_proxy().build()?;
    let (sender, _receiver) = tokio::sync::mpsc::channel(14);
    engine::analysis::download(engine::analysis::DownloadInput {
        client: &client,
        url,
        sender,
        headers: Duration::from_secs(8),
        read_timeout: Duration::from_secs(10),
    })
    .await
}
