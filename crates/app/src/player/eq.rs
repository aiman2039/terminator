#[cfg(test)]
use super::engine;
#[cfg(test)]
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

pub(super) fn format_clock(duration: Duration) -> String {
    let total = duration.as_secs();
    format!("{}:{:02}", total / 60, total % 60)
}

#[cfg(test)]
#[cfg(unix)]
pub(crate) async fn fixture_download(url: String) -> Result<()> {
    let client = reqwest::Client::builder().no_proxy().build()?;
    let (sender, _receiver) = tokio::sync::mpsc::channel(14);
    engine::analysis::download(
        &client,
        url,
        sender,
        Duration::from_secs(8),
        Duration::from_secs(10),
    )
    .await
}
