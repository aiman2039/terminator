pub(super) mod close_idle;
#[cfg(feature = "test-support")]
pub(super) mod diagnostics;
pub(super) mod exit;
#[cfg(any(test, target_os = "macos"))]
pub(super) mod merge;
pub(super) mod notify_test;
pub(super) mod resource_sample;
pub(super) mod retry_budget;
#[cfg(any(windows, test))]
pub(super) mod windows_updates;
