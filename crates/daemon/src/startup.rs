use super::*;
use crate::server::HistoryJob;
use std::{io::Write, sync::atomic::AtomicBool, sync::mpsc};

pub(crate) struct Bootstrap {
    pub(crate) paths: Paths,
    pub(crate) catalog_paths: Option<Paths>,
    pub(crate) _legacy_guard: Option<fs::File>,
    pub(crate) _lock: fs::File,
}

pub(crate) fn bootstrap() -> Result<Bootstrap> {
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
    Ok(Bootstrap {
        paths,
        catalog_paths,
        _legacy_guard,
        _lock: lock,
    })
}

pub(crate) fn open_state(
    paths: &Paths,
    catalog_paths: &Option<Paths>,
) -> Result<(storage::Store, State)> {
    let (mut store, mut state) = storage::Store::open(paths)?;
    terminator_core::ssh::set_tuning(state.settings.ssh_tuning());
    recover_loaded(&mut state);
    drop_non_resumable(paths, &mut state);
    if let Some(root) = &catalog_paths {
        bind_generation(root, &mut store, &mut state)?;
    }
    advertise(&mut state, catalog_paths.is_some())?;
    let credential_paths = catalog_paths.as_ref().unwrap_or(paths);
    let pending = vnc_session::credentials::migrate(credential_paths, &mut state.vnc_profiles);
    ensure!(
        pending == 0,
        "{pending} legacy VNC passwords could not be migrated; unlock the OS credential store before restarting. Existing saved state was not overwritten"
    );
    if let Some(root) = &catalog_paths {
        let catalog = generations::Catalog::open(root)?;
        if catalog.active()?.as_deref() == Some(&state.generation) {
            catalog.save_workspace(&state)?;
        }
    }
    store.save(&state)?;
    Ok((store, state))
}

fn recover_loaded(state: &mut State) {
    // Store::open may report a quarantined store or reset settings via the
    // degraded flag; recover() clears runtime health, so keep that note.
    let load_note = state.degraded.clone();
    state.recover();
    if load_note.is_some() {
        state.degraded = load_note;
    }
}

/// Migrate existing stores: drop historical sessions without an agent
/// resume command so old plain shells/editors stop filling History.
fn drop_non_resumable(paths: &Paths, state: &mut State) {
    let removed = state.prune_non_resumable_ended();
    if !removed.is_empty()
        && let Ok(mut history) = storage::History::new(paths.clone())
    {
        for id in &removed {
            let _ = history.clear(Some(id), true);
        }
        let _ = history.flush();
    }
}

/// A catalog-managed daemon owns a brand-new generation directory.
fn bind_generation(root: &Paths, store: &mut storage::Store, state: &mut State) -> Result<()> {
    let owner = std::env::var("TERMINATOR_GENERATION")?;
    ensure!(
        state.sessions.is_empty(),
        "Generation directory must be new; never restart owned processes"
    );
    state.generation.clone_from(&owner);
    store.bind_owner(owner);
    generations::Catalog::open(root)?.refresh(state)?;
    Ok(())
}

/// Stamps the version, build and capabilities clients gate requests on.
fn advertise(state: &mut State, catalog: bool) -> Result<()> {
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
        VNC_CAPABILITY.into(),
        VNC_REGIONS_CAPABILITY.into(),
        terminator_core::remote::CAPABILITY.into(),
        terminator_core::ssh::HOST_KEYS_CAPABILITY.into(),
        terminator_core::remote::HOSTS_CAPABILITY.into(),
        terminator_core::remote::FORWARDING_CAPABILITY.into(),
        terminator_core::remote::TRANSFER_CAPABILITY.into(),
        terminator_core::remote::EDIT_CAPABILITY.into(),
        terminator_core::ssh::TUNING_CAPABILITY.into(),
        vnc::FEATURES_CAPABILITY.into(),
        vnc::LABELS_CAPABILITY.into(),
        vnc::ERRORS_CAPABILITY.into(),
        vnc::START_COMMAND_CAPABILITY.into(),
        vnc::SHM_CAPABILITY.into(),
        vnc::DETACH_TIMEOUT_CAPABILITY.into(),
    ];
    if catalog {
        state.daemon_build = Some(generations::build_identity(
            std::env::current_exe()?
                .parent()
                .context("Missing executable directory")?,
        )?);
        state.daemon_catalog_version = Some(generations::CATALOG_VERSION);
        state.capabilities.push(generations::CAPABILITY.into());
    }
    Ok(())
}

/// Removes helpers and shared VNC framebuffers left by crashed daemons and
/// stages this one's helper.
pub(crate) fn stage_helper(paths: &Paths) -> Result<helper::Helper> {
    helper::cleanup_abandoned(&paths.data)?;
    crate::vnc_session::shm::remove_stale_files(&paths.runtime);
    helper::Helper::stage(
        &std::env::current_exe()?.with_file_name(exe_name("terminator-hook")),
        &paths.data,
    )
}

pub(crate) fn bind_listener(paths: &Paths) -> Result<transport::Listener> {
    let listener = transport::Listener::bind_ipc(&paths.socket())?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

/// Writes a fresh client auth token and returns it.
pub(crate) fn publish_auth(paths: &Paths) -> Result<String> {
    let auth = id();
    atomic_write(&paths.auth(), auth.as_bytes())?;
    Ok(auth)
}

pub(crate) struct StartInput {
    pub(crate) helper: helper::Helper,
    pub(crate) catalog_paths: Option<Paths>,
    pub(crate) store: storage::Store,
    pub(crate) state: State,
    pub(crate) paths: Paths,
    pub(crate) auth: String,
}

/// The shared daemon state and the receivers its workers drain.
pub(crate) struct Started {
    pub(crate) shared: Arc<Shared>,
    pub(crate) history_rx: mpsc::Receiver<HistoryJob>,
    pub(crate) alert_rx: mpsc::Receiver<String>,
}

pub(crate) fn start_shared(input: StartInput) -> Started {
    let StartInput {
        helper,
        catalog_paths,
        store,
        state,
        paths,
        auth,
    } = input;
    let (history, history_rx) = mpsc::sync_channel::<HistoryJob>(512);
    let (alerts, alert_rx) = mpsc::sync_channel::<String>(64);
    let shared = Arc::new(Shared {
        helper,
        catalog_paths,
        state: Mutex::new(state),
        store: Mutex::new(store),
        sessions: Mutex::new(HashMap::new()),
        vnc_sessions: Mutex::new(HashMap::new()),
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
        forwards: Mutex::new(crate::remote::Forwards::default()),
        remote_transfers: Mutex::new(crate::remote::Transfers::default()),
    });
    Started {
        shared,
        history_rx,
        alert_rx,
    }
}

/// The accept loop's wake socketpair. `tx` stays alive for the daemon's
/// lifetime; the notification waker writes through a clone of it.
pub(crate) struct Waker {
    _tx: transport::Stream,
    pub(crate) rx: transport::Stream,
    /// Draining through a clone: the poll set borrows `rx` while a wake
    /// needs a mutable drain handle on the same socket buffer.
    pub(crate) drain: transport::Stream,
}

/// A socketpair wakes the accept loop when a notification waiter needs
/// Cocoa run-loop pumping or has finished.
pub(crate) fn install_waker() -> Result<Waker> {
    let (wake_tx, wake_rx) = transport::pair()?;
    wake_tx.set_nonblocking(true)?;
    wake_rx.set_nonblocking(true)?;
    let wake_drain = wake_rx.try_clone()?;
    notifications::set_waker({
        let wake_tx = Mutex::new(wake_tx.try_clone()?);
        move || {
            if let Ok(mut guard) = wake_tx.lock() {
                let _ = guard.write(&[1u8]);
            }
        }
    });
    Ok(Waker {
        _tx: wake_tx,
        rx: wake_rx,
        drain: wake_drain,
    })
}
