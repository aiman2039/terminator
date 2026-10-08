//! Async HTTP -> bounded compressed bytes -> native decoder -> prepared PCM.
pub(crate) mod analysis;
mod decode;
#[cfg(test)]
mod engine_tests;
mod handle;
mod pipeline;
pub(crate) use handle::{Handle, Outcome, Playable, Status};
