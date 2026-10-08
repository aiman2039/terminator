#![forbid(unsafe_code)]
mod editor;
mod helper;
mod idle_close;
mod notifications;
mod ntfy;
mod presence;
mod review;
mod terminal_env;
mod terminal_events;

#[derive(Clone, Copy)]
enum Launch {
    Shell,
    Editor,
    Review { staged: bool },
}
mod shell;
mod storage;
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
#[cfg(unix)]
use rustix::event::{PollFd, PollFlags, Timespec, poll};
#[cfg(unix)]
use rustix::io::Errno;
use std::{
    collections::HashMap,
    fs,
    io::Write,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use terminator_core::*;

mod server;
use server::{HistoryJob, Shared, drain_wake, relock, serve};

fn main() -> Result<()> {
    notifications::initialize();
    let mut paths = Paths::discover()?;
    if let Some(p) = std::env::var_os("TERMINATOR_RUNTIME_DIR") {
        paths.runtime = p.into();
    }
    paths.init()?;
    crash::install(crash::CrashInstall {
        binary: "terminator-daemon",
        version: env!("CARGO_PKG_VERSION"),
        data_dir: paths.data.clone(),
        notify: None,
    });
    let catalog_paths = if let Some(data) = std::env::var_os("TERMINATOR_CATALOG_DATA") {
        Some(Paths {
            data: data.into(),
            runtime: std::env::var_os("TERMINATOR_CATALOG_RUNTIME")
                .context("Catalog runtime")?
                .into(),
        })
    } else {
        None
    };
    if let Some(root) = &catalog_paths {
        generations::validate_endpoint(
            root,
            &paths,
            &std::env::var("TERMINATOR_GENERATION").context("Missing generation identity")?,
        )?;
    }
    let _legacy_guard = if let Some(root) = &catalog_paths {
        let guard = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.runtime.join("daemon.lock"))?;
        FileExt::try_lock_shared(&guard).context("Legacy service has not retired")?;
        Some(guard)
    } else {
        None
    };
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(paths.runtime.join("daemon.lock"))?;
    lock.try_lock_exclusive()
        .context("Daemon already running")?;
    helper::cleanup_abandoned(&paths.data)?;
    let helper = helper::Helper::stage(
        &std::env::current_exe()?.with_file_name(exe_name("terminator-hook")),
        &paths.data,
    )?;
    let listener = transport::Listener::bind_ipc(&paths.socket())?;
    listener.set_nonblocking(true)?;
    let auth = id();
    atomic_write(&paths.auth(), auth.as_bytes())?;
    let (mut store, mut state) = storage::Store::open(&paths)?;
    // Store::open may report a quarantined store or reset settings via the
    // degraded flag; recover() clears runtime health, so keep that note.
    let load_note = state.degraded.clone();
    state.recover();
    if load_note.is_some() {
        state.degraded = load_note;
    }
    // Migrate existing stores: drop historical sessions without an agent
    // resume command so old plain shells/editors stop filling History.
    let removed = state.prune_non_resumable_ended();
    if !removed.is_empty()
        && let Ok(mut history) = storage::History::new(paths.clone())
    {
        for id in &removed {
            let _ = history.clear(Some(id), true);
        }
        let _ = history.flush();
    }
    if let Some(root) = &catalog_paths {
        let owner = std::env::var("TERMINATOR_GENERATION")?;
        ensure!(
            state.sessions.is_empty(),
            "Generation directory must be new; never restart owned processes"
        );
        state.generation.clone_from(&owner);
        store.bind_owner(owner);
        generations::Catalog::open(root)?.refresh(&mut state)?;
    }
    state.daemon_version = Some(env!("CARGO_PKG_VERSION").into());
    state.capabilities = vec![
        terminator_core::idle_close::CAPABILITY.into(),
        STABLE_HELPER_CAPABILITY.into(),
        SHUTDOWN_IF_IDLE_CAPABILITY.into(),
        snapshot::CAPABILITY.into(),
        NVIM_REVIEW_CAPABILITY.into(),
        TERMINAL_NOTICES_CAPABILITY.into(),
        NOTIFICATION_SOUND_CAPABILITY.into(),
        NTFY_CAPABILITY.into(),
        DIFF_CLOSE_SETTINGS_CAPABILITY.into(),
        WORKTREES_CAPABILITY.into(),
        SCREEN_CAPABILITY.into(),
        METADATA_SETTINGS_CAPABILITY.into(),
        AGENT_PRESENCE_CAPABILITY.into(),
    ];
    if catalog_paths.is_some() {
        state.daemon_build = Some(generations::build_identity(
            std::env::current_exe()?
                .parent()
                .context("Missing executable directory")?,
        )?);
        state.daemon_catalog_version = Some(generations::CATALOG_VERSION);
        state.capabilities.push(generations::CAPABILITY.into());
    }
    store.save(&state)?;
    let (history, history_rx) = mpsc::sync_channel::<HistoryJob>(512);
    let (alerts, alert_rx) = mpsc::sync_channel::<String>(64);
    let shared = Arc::new(Shared {
        helper,
        catalog_paths,
        state: Mutex::new(state),
        store: Mutex::new(store),
        sessions: Mutex::new(HashMap::new()),
        paths,
        auth,
        focused: Mutex::new((false, Instant::now())),
        worktree_operations: Mutex::new(()),
        terminal_operations: Mutex::new(()),
        shutdown: AtomicBool::new(false),
        history,
        alerts,
        ntfy: ntfy::start(),
        ntfy_cooldown: Mutex::new(ntfy::Cooldown::default()),
    });
    let weak = Arc::downgrade(&shared);
    thread::spawn(move || {
        let Some(shared) = weak.upgrade() else { return };
        let mut history = match storage::History::new(shared.paths.clone()) {
            Ok(h) => h,
            Err(e) => {
                shared.degraded(&format!("History initialization failed: {e}"));
                return;
            }
        };
        drop(shared);
        let mut last_flush = Instant::now();
        let mut last_prune = Instant::now();
        let mut previous_settings = None;
        loop {
            let Some(s) = weak.upgrade() else { break };
            match history_rx.recv_timeout(Duration::from_millis(250)) {
                Ok(HistoryJob::Prune(budget, tx)) => {
                    let settings = relock(&s.state).settings.clone();
                    let _ = tx.send(
                        history
                            .prune_with_budget(&settings, budget)
                            .map_err(|e| e.to_string()),
                    );
                }
                Ok(HistoryJob::Append(sid, data)) => {
                    if let Err(e) = history.append(&sid, &data) {
                        let mut state = relock(&s.state);
                        if let Some(record) = state.sessions.iter_mut().find(|r| r.id == sid) {
                            record.truncated = true;
                        }
                        state.degraded = Some(format!(
                            "History write failed; some terminal history was not saved: {e}"
                        ));
                        state.revision = state.revision.saturating_add(1);
                    }
                }
                Ok(HistoryJob::Flush(tx)) => {
                    let _ = tx.send(history.flush().map_err(|e| e.to_string()));
                }
                Ok(HistoryJob::Clear(session, remove, tx)) => {
                    let _ = tx.send(
                        history
                            .clear(session.as_deref(), remove)
                            .map_err(|e| e.to_string()),
                    );
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                _ => {}
            }
            if last_flush.elapsed() >= Duration::from_millis(250) {
                if let Err(e) = history.flush() {
                    s.degraded(&format!("History flush failed: {e}"));
                }
                last_flush = Instant::now();
            }
            let settings = relock(&s.state).settings.clone();
            let limits = (
                settings.history_days,
                settings.session_mib,
                settings.total_mib,
            );
            if history.exceeds(&settings)
                || previous_settings != Some(limits)
                || last_prune.elapsed() >= Duration::from_secs(30)
            {
                match history.prune(&settings) {
                    Ok(ids) if !ids.is_empty() => {
                        let mut state = relock(&s.state);
                        for record in &mut state.sessions {
                            if ids.contains(&record.id) {
                                record.truncated = true;
                            }
                        }
                        state.revision = state.revision.saturating_add(1);
                        drop(state);
                        let _ = s.persist();
                    }
                    Ok(_) => {}
                    Err(e) => s.degraded(&format!("History cleanup failed: {e}")),
                }
                previous_settings = Some(limits);
                last_prune = Instant::now();
            }
            if s.shutdown.load(Ordering::Relaxed) {
                break;
            }
        }
        let _ = history.flush();
    });
    let weak = Arc::downgrade(&shared);
    thread::spawn(move || {
        let mut last = Instant::now()
            .checked_sub(Duration::from_secs(10))
            .unwrap_or_else(Instant::now);
        while let Ok(nid) = alert_rx.recv() {
            let Some(s) = weak.upgrade() else { break };
            if std::env::var_os("TERMINATOR_NO_NOTIFICATIONS").is_some()
                || last.elapsed() < Duration::from_secs(2)
            {
                continue;
            }
            let state = relock(&s.state);
            let summary = state
                .notifications
                .iter()
                .find(|n| n.id == nid)
                .map(|n| format!("Agent: {}", n.state.label()))
                .or_else(|| {
                    state
                        .terminal_notices
                        .iter()
                        .find(|n| n.id == nid && !n.dismissed)
                        .map(|n| format!("Terminal: {}", n.title))
                });
            let Some(summary) = summary else {
                continue;
            };
            let sound = state.settings.notification_sound;
            drop(state);
            notifications::send(notifications::DesktopAlert {
                paths: s.catalog_paths.clone().unwrap_or_else(|| s.paths.clone()),
                summary,
                notice: nid,
                sound,
            });
            last = Instant::now();
        }
    });
    let weak = Arc::downgrade(&shared);
    thread::spawn(move || {
        let born = Instant::now();
        let mut global_prune = Instant::now()
            .checked_sub(Duration::from_secs(30))
            .unwrap_or_else(Instant::now);
        loop {
            thread::sleep(Duration::from_secs(1));
            let Some(shared) = weak.upgrade() else {
                break;
            };
            if shared.shutdown.load(Ordering::Acquire) {
                break;
            }
            let result = (|| -> Result<()> {
                if let Some(root) = &shared.catalog_paths {
                    let _coordination = generations::coordinate(root)?;
                    let catalog = generations::Catalog::open(root)?;
                    let owner = relock(&shared.state).generation.clone();
                    catalog.refresh(&mut relock(&shared.state))?;
                    if catalog.active()?.as_deref() == Some(&owner)
                        && !shared.shutdown.load(Ordering::Acquire)
                    {
                        let _ = catalog.restore_serving();
                    }
                    let owners = catalog.generations()?;
                    let draining = owners.iter().any(|g| {
                        g.id == owner
                            && (g.status == generations::Status::Draining
                                || (g.status == generations::Status::Prepared
                                    && born.elapsed() > Duration::from_secs(30)))
                    });
                    let active_owner = catalog.active()?.as_deref() == Some(&owner);
                    drop(_coordination);
                    if active_owner {
                        if global_prune.elapsed() >= Duration::from_secs(30) {
                            if let Err(error) = shared.prune_global(&owners) {
                                shared.degraded(&format!("Global history retention: {error:#}"));
                            }
                            if let Err(error) =
                                generations::prune_retired(root, generations::RETIRED_RETENTION)
                            {
                                shared.degraded(&format!("Generation retention: {error:#}"));
                            }
                            global_prune = Instant::now();
                        }
                        for other in owners
                            .iter()
                            .filter(|g| g.id != owner && g.status != generations::Status::Retired)
                        {
                            let _ = generations::recover_exited(root, other);
                        }
                    }
                    if draining
                        && !relock(&shared.state)
                            .sessions
                            .iter()
                            .any(|s| s.lifecycle.live())
                    {
                        let _ = shared.handle(Request::RetireIfDraining);
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                shared.degraded(&format!("Generation maintenance: {error:#}"));
            }
        }
    });
    presence::start(Arc::downgrade(&shared));
    let active = Arc::new(AtomicUsize::new(0));
    // Event-driven accept loop: the main thread blocks in `poll` with no
    // timeout while no notification waiter exists, so an idle daemon does no
    // periodic work. A socketpair wakes it when a waiter needs Cocoa run-loop
    // pumping or has finished; a shutdown request wakes it explicitly. The
    // listener is still non-blocking so the poll/accept pair never blocks.
    // Windows has no `poll` on sockets here; the loop below polls the
    // non-blocking listener and waker on a short sleep instead.
    let (wake_tx, wake_rx) = transport::pair()?;
    wake_tx.set_nonblocking(true)?;
    wake_rx.set_nonblocking(true)?;
    // Draining through a clone: the poll set borrows `wake_rx` while a wake
    // needs a mutable drain handle on the same socket buffer.
    let mut wake_drain = wake_rx.try_clone()?;
    notifications::set_waker({
        let wake_tx = Mutex::new(wake_tx.try_clone()?);
        move || {
            if let Ok(mut guard) = wake_tx.lock() {
                let _ = guard.write(&[1u8]);
            }
        }
    });
    #[cfg(unix)]
    while !shared.shutdown.load(Ordering::Relaxed) {
        let waiting = notifications::waiting();
        // While a waiter exists, service the Cocoa run loop often enough that
        // click callbacks and the 0.5 s dismiss poll are not starved, without
        // the previous ~100 wakeups/s. The pump itself is a 1 ms slice.
        let timeout = waiting.then_some(Timespec {
            tv_sec: 0,
            tv_nsec: 100_000_000,
        });
        let mut fds = [
            PollFd::new(listener.as_std(), PollFlags::IN),
            PollFd::new(wake_rx.as_std(), PollFlags::IN),
        ];
        match poll(&mut fds, timeout.as_ref()) {
            Ok(0) => notifications::pump(),
            Ok(_) => {
                if fds[1].revents().contains(PollFlags::IN) {
                    drain_wake(&mut wake_drain);
                }
                if fds[0].revents().contains(PollFlags::IN) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            if active.load(Ordering::Relaxed) <= 256 {
                                let s = Arc::clone(&shared);
                                let count = Arc::clone(&active);
                                count.fetch_add(1, Ordering::Relaxed);
                                thread::spawn(move || {
                                    // A panicking request kills at most its
                                    // own connection; the daemon keeps
                                    // serving. (The crash hook only dumps
                                    // main-thread panics, so this stays quiet
                                    // apart from the line below.)
                                    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                                        || serve(stream, s),
                                    ))
                                    .is_err()
                                    {
                                        eprintln!(
                                            "daemon connection handler panicked; connection dropped"
                                        );
                                    }
                                    count.fetch_sub(1, Ordering::Relaxed);
                                });
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(e) => return Err(e.into()),
                    }
                }
                if waiting {
                    notifications::pump();
                }
            }
            Err(Errno::INTR) => notifications::pump(),
            Err(e) => return Err(e.into()),
        }
    }
    #[cfg(not(unix))]
    while !shared.shutdown.load(Ordering::Relaxed) {
        drain_wake(&mut wake_drain);
        if notifications::waiting() {
            notifications::pump();
        }
        match listener.accept() {
            Ok((stream, _)) => {
                if active.load(Ordering::Relaxed) <= 256 {
                    let s = Arc::clone(&shared);
                    let count = Arc::clone(&active);
                    count.fetch_add(1, Ordering::Relaxed);
                    thread::spawn(move || {
                        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            serve(stream, s)
                        }))
                        .is_err()
                        {
                            eprintln!("daemon connection handler panicked; connection dropped");
                        }
                        count.fetch_sub(1, Ordering::Relaxed);
                    });
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e.into()),
        }
        std::thread::sleep(Duration::from_millis(10));
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
    Ok(())
}
