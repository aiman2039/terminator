#![forbid(unsafe_code)]
mod accept;
mod console;
mod editor;
mod helper;
mod idle_close;
mod notifications;
mod ntfy;
mod presence;
mod review;
mod terminal_env;
mod terminal_events;
mod vnc_session;
mod workers;

#[derive(Clone, Copy)]
enum Launch {
    Shell,
    Editor,
    Review { staged: bool },
}
mod remote;
mod shell;
mod storage;
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use std::{
    collections::HashMap,
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use terminator_core::*;

mod server;
mod startup;
use server::{Shared, relock};
use startup::{Bootstrap, StartInput, bootstrap, open_state};

fn main() -> Result<()> {
    accept::raise_fd_limit();
    notifications::initialize();
    let Bootstrap {
        paths,
        catalog_paths,
        _legacy_guard,
        _lock,
    } = bootstrap()?;
    let helper = startup::stage_helper(&paths)?;
    let listener = startup::bind_listener(&paths)?;
    let auth = startup::publish_auth(&paths)?;
    let (store, state) = open_state(&paths, &catalog_paths)?;
    let started = startup::start_shared(StartInput {
        helper,
        catalog_paths,
        store,
        state,
        paths,
        auth,
    });
    let shared = started.shared;
    vnc_session::credentials::finish_activation(Arc::downgrade(&shared));
    workers::HistoryWorker::spawn(Arc::downgrade(&shared), started.history_rx);
    workers::AlertWorker::spawn(Arc::downgrade(&shared), started.alert_rx);
    workers::GenerationWorker::spawn(Arc::downgrade(&shared));
    presence::start(Arc::downgrade(&shared));
    let active = Arc::new(AtomicUsize::new(0));
    let mut wake = startup::install_waker()?;
    accept::AcceptLoop {
        listener: &listener,
        shared: &shared,
        active: &active,
        wake_drain: &mut wake.drain,
        wake_rx: &wake.rx,
    }
    .run()?;
    shutdown(&shared, &active)
}

fn shutdown(shared: &Arc<Shared>, active: &Arc<AtomicUsize>) -> Result<()> {
    // Shutdown is set by a connection handler before it writes the reply.
    // Keep the process alive until accepted requests finish, or a fast main
    // loop can exit while the shutdown client is still reading its frame.
    // Bound draining: a stalled client must not prevent daemon exit.
    let drain_deadline = Instant::now()
        .checked_add(Duration::from_secs(3))
        .unwrap_or_else(Instant::now);
    while active.load(Ordering::Acquire) != 0 && Instant::now() < drain_deadline {
        thread::sleep(Duration::from_millis(10));
    }
    shared.persist()?;
    transport::cleanup_ipc(&shared.paths.socket());
    shared.helper.cleanup()?;
    if shared.catalog_paths.is_some() {
        let _ = fs::remove_file(shared.paths.auth());
        let bin = shared.paths.data.join("bin");
        if bin.is_dir() {
            fs::remove_dir_all(bin)?;
        }
    }
    if let Some(root) = &shared.catalog_paths {
        let live = relock(&shared.state)
            .sessions
            .iter()
            .any(|s| s.lifecycle.live());
        if !live {
            let _coordination = generations::coordinate(root)?;
            generations::Catalog::open(root)?.retire(&relock(&shared.state).generation)?;
        }
    }
    shared.exit_ssh_controls();
    Ok(())
}
