//! GUI exit, idle close, resource sampling and diagnostics.
pub(super) mod close_idle;
#[cfg(feature = "test-support")]
pub(super) mod diagnostics;
pub(super) mod exit;
pub(super) mod notify_test;
pub(super) mod resource_sample;
pub(super) mod retry_budget;
