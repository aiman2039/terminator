//! GUI-only Winamp-style player. Audio dies with the GUI.
mod engine;
mod playlist;
pub(super) mod radio;
mod tap;
mod ui;

use super::*;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub use radio::parse_stream_url;

/// Low 64 bits, matching `as u64`.
fn u64_from_u128(value: u128) -> u64 {
    u64::try_from(value)
        .unwrap_or_else(|_| u64::try_from(value & u128::from(u64::MAX)).unwrap_or(0))
}

/// Truncating `as u64`. Non-finite or negative becomes 0; overflow saturates.
fn u64_from_f32(value: f32) -> u64 {
    if value.is_nan() || value <= 0.0 {
        return 0;
    }
    if !value.is_finite() || value >= 18_446_744_073_709_551_616.0 {
        return u64::MAX;
    }
    let bits = value.to_bits();
    let Some(exp) = i32::try_from((bits >> 23) & 0xff)
        .ok()
        .and_then(|biased| biased.checked_sub(127))
    else {
        return 0;
    };
    if exp < 0 {
        return 0;
    }
    let mantissa = u64::from((bits & 0x007f_ffff) | (1_u32 << 23));
    let Some(shift) = exp.checked_sub(23) else {
        return 0;
    };
    if shift >= 0 {
        let Ok(places) = u32::try_from(shift) else {
            return u64::MAX;
        };
        mantissa.checked_shl(places).unwrap_or(u64::MAX)
    } else {
        mantissa.checked_shr(shift.unsigned_abs()).unwrap_or(0)
    }
}

pub(super) fn audio_from_dir(root: &Path) -> Vec<PathBuf> {
    playlist::collect_audio(root, playlist::AUDIO_WALK_CAP)
}

struct PlayerIconButton<'a> {
    name: &'a str,
    tip: &'a str,
    lit: bool,
    size: f32,
    glyph: f32,
}

const SAMPLE_TRACKS: &[(&str, &[u8])] = &[
    ("pulse.wav", include_bytes!("../../assets/audio/pulse.wav")),
    ("hum.wav", include_bytes!("../../assets/audio/hum.wav")),
    ("chime.wav", include_bytes!("../../assets/audio/chime.wav")),
];

pub(super) fn install_samples(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if std::fs::create_dir_all(dir).is_err() {
        return out;
    }
    for (name, bytes) in SAMPLE_TRACKS {
        let path = dir.join(name);
        if !path.exists() && std::fs::write(&path, bytes).is_err() {
            continue;
        }
        if path.is_file() {
            out.push(path);
        }
    }
    out
}

pub(super) const AUDIO_EXTENSIONS: &[&str] = &["mp3", "flac", "ogg", "wav", "m4a", "opus", "aac"];

pub fn supported(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            AUDIO_EXTENSIONS
                .iter()
                .any(|allowed| ext.eq_ignore_ascii_case(allowed))
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stack {
    Add,
    Rem,
    Sel,
    Misc,
    List,
    Stations,
}

mod app_player;
mod controller;
mod eq;
#[cfg(test)]
mod player_tests;
pub(crate) use controller::{Controller, PollInput, Skip};
#[cfg(test)]
#[cfg(unix)]
pub(crate) use eq::fixture_download;
