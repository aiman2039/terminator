//! SIGTERM (for example at logout): save pending scrollback and deferred
//! hook state, then end the way SIGTERM would have ended the process.
//! Without this, up to 250 ms of history and 1 s of hook state were lost.

use std::sync::Weak;

use crate::server::Shared;

/// Saves what is still in memory. Each step is bounded (history flush
/// waits at most 3 s) and best effort: the process ends regardless.
#[cfg(unix)]
fn save_before_exit(shared: &Shared) {
    let _ = shared.flush_history();
    let _ = shared.persist();
}

#[cfg(unix)]
pub(crate) fn install(weak: Weak<Shared>) {
    use signal_hook::consts::SIGTERM;
    let Ok(mut signals) = signal_hook::iterator::Signals::new([SIGTERM]) else {
        return;
    };
    let _ = std::thread::Builder::new()
        .name("sigterm".into())
        .spawn(move || {
            if signals.forever().next().is_none() {
                return;
            }
            if let Some(shared) = weak.upgrade() {
                save_before_exit(&shared);
            }
            // Same exit as an unhandled SIGTERM, so callers see no change.
            let _ = signal_hook::low_level::emulate_default_handler(SIGTERM);
            std::process::exit(143);
        });
}

#[cfg(not(unix))]
pub(crate) fn install(_weak: Weak<Shared>) {}
