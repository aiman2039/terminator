use super::*;

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
        VNC_CAPABILITY.into(),
        VNC_REGIONS_CAPABILITY.into(),
        terminator_core::remote::CAPABILITY.into(),
        terminator_core::ssh::HOST_KEYS_CAPABILITY.into(),
        terminator_core::remote::HOSTS_CAPABILITY.into(),
        terminator_core::remote::FORWARDING_CAPABILITY.into(),
        terminator_core::remote::TRANSFER_CAPABILITY.into(),
        vnc::FEATURES_CAPABILITY.into(),
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
