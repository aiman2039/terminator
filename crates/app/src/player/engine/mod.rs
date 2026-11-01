//! Async HTTP -> bounded compressed bytes -> native decoder -> prepared PCM.
pub(crate) mod analysis;
pub(crate) use analysis::DownloadInput;
mod decode;
#[cfg(test)]
mod engine_tests;
pub(crate) mod equalizer;
mod handle;
mod pipeline;
pub(crate) use equalizer::EqSettings;
pub(crate) use handle::{Handle, Outcome, Playable, Status};
