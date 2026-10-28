use super::*;
pub(super) struct SpawnVncThreadInput {
    pub runtime: Arc<Mutex<VncRuntime>>,
    pub session_id: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub shared: bool,
    pub options: terminator_core::vnc::VncOptions,
    pub input_rx: mpsc::Receiver<VncInputEvent>,
}
impl VncConnectSpec {
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.host.is_empty() && self.host.len() <= 255,
            "Invalid VNC host"
        );
        ensure!(self.port > 0, "Invalid VNC port");
        ensure!(
            self.password.len() <= 4096
                && self.username.len() <= 4096
                && !self.username.chars().any(char::is_control),
            "VNC credentials exceed their supported limits"
        );
        self.options.validate()?;
        Ok(())
    }
}

impl Shared {
    pub(crate) fn vnc_connect(self: &Arc<Self>, mut spec: VncConnectSpec) -> Result<VncSession> {
        let host = spec.host.trim().to_owned();
        spec.host = host;
        spec.validate()?;
        self.apply_saved_endpoint(&mut spec)?;
        spec.options.validate()?;
        let sid = id();
        let (record, stale) = self.record_vnc_session(&mut spec, &sid)?;
        if let Some(paths) = &self.catalog_paths {
            let _ = terminator_core::generations::Catalog::open(paths)
                .and_then(|c| c.save_workspace(&relock(&self.state)));
        }
        let _ = self.persist();

        self.clear_stale_vnc_runtimes(&stale);
        self.start_vnc_runtime(sid, spec);
        Ok(record)
    }

    /// A saved profile pins the endpoint and supplies its options and,
    /// unless one was typed, the stored password.
    fn apply_saved_endpoint(&self, spec: &mut VncConnectSpec) -> Result<()> {
        let saved_profile = spec.profile_id.as_ref().and_then(|pid| {
            relock(&self.state)
                .vnc_profiles
                .iter()
                .find(|p| &p.id == pid)
                .cloned()
        });
        let Some(profile) = &saved_profile else {
            return Ok(());
        };
        ensure!(
            profile.host == spec.host && profile.port == spec.port,
            "Saved credentials cannot be used for another VNC endpoint"
        );
        spec.options = profile.options.clone();
        if spec.password.is_empty() && profile.credential_saved {
            spec.password =
                credentials::load(self.catalog_paths.as_ref().unwrap_or(&self.paths), profile)?;
        }
        Ok(())
    }

    /// Adds the connecting session's record and returns it with the ids of
    /// the dead duplicates it replaced.
    fn record_vnc_session(
        &self,
        spec: &mut VncConnectSpec,
        sid: &str,
    ) -> Result<(VncSession, Vec<String>)> {
        let now_ts = now();
        let mut state = relock(&self.state);
        validate_project(&state, &spec.project)?;
        let stale = Self::drop_dead_duplicates(&mut state, spec);
        Self::merge_saved_profile(&mut state, spec, now_ts);
        let label = spec
            .label
            .take()
            .map(|l| l.trim().to_owned())
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| format!("{}:{}", spec.host, spec.port));
        let record = VncSession {
            options: spec.options.clone(),
            retry_attempt: 0,
            retry_in_seconds: 0,
            id: sid.to_owned(),
            project_id: std::mem::take(&mut spec.project),
            profile_id: spec.profile_id.take(),
            label,
            host: spec.host.clone(),
            port: spec.port,
            width: 0,
            height: 0,
            view_only: spec.view_only,
            status: VncSessionStatus::Connecting,
            error: None,
            created: now_ts,
            generation: state.generation.clone(),
        };
        state.vnc_sessions.push(record.clone());
        state.revision = state.revision.saturating_add(1);
        Ok((record, stale))
    }

    /// A reconnect to the same endpoint leaves the previous dead record
    /// behind forever; drop it (and its runtime) so the sidebar and the
    /// session list do not fill with duplicates.
    fn drop_dead_duplicates(
        state: &mut terminator_core::State,
        spec: &VncConnectSpec,
    ) -> Vec<String> {
        let mut stale: Vec<String> = Vec::new();
        state.vnc_sessions.retain(|session| {
            let duplicate = session.project_id == spec.project
                && session.host == spec.host
                && session.port == spec.port
                && !session.status.live();
            if duplicate {
                stale.push(session.id.clone());
            }
            !duplicate
        });
        stale
    }

    /// Fills missing credentials and the sharing flags from the saved
    /// profile, and stamps its last connection time.
    fn merge_saved_profile(
        state: &mut terminator_core::State,
        spec: &mut VncConnectSpec,
        now_ts: u64,
    ) {
        let Some(pid) = &spec.profile_id else {
            return;
        };
        let Some(profile) = state.vnc_profiles.iter_mut().find(|p| &p.id == pid) else {
            return;
        };
        if spec.password.is_empty() {
            spec.password.clone_from(&profile.password);
        }
        if spec.username.is_empty() {
            spec.username.clone_from(&profile.username);
        }
        spec.shared = profile.shared;
        spec.view_only = profile.view_only;
        profile.last_connected = now_ts;
    }

    fn start_vnc_runtime(self: &Arc<Self>, sid: String, spec: VncConnectSpec) {
        let VncConnectSpec {
            options,
            project: _,
            profile_id: _,
            host,
            port,
            username,
            password,
            shared,
            view_only,
            label: _,
        } = spec;
        let (input_tx, input_rx) = mpsc::channel::<VncInputEvent>(256);
        let runtime = Arc::new(Mutex::new(VncRuntime::new(view_only, input_tx)));
        relock(&runtime).options = options.clone();
        relock(&self.vnc_sessions).insert(sid.clone(), Arc::clone(&runtime));
        self.spawn_vnc_thread(SpawnVncThreadInput {
            runtime,
            session_id: sid,
            host,
            port,
            username,
            password,
            shared,
            options,
            input_rx,
        });
    }

    fn clear_stale_vnc_runtimes(&self, stale: &[String]) {
        for id in stale {
            if let Some(runtime) = relock(&self.vnc_sessions).remove(id) {
                let mut rt = relock(&runtime);
                rt.input_tx = None;
                rt.status = VncSessionStatus::Disconnected;
                rt.revision = rt.revision.saturating_add(1);
            }
        }
    }
    fn spawn_vnc_thread(self: &Arc<Self>, input: SpawnVncThreadInput) {
        let SpawnVncThreadInput {
            runtime,
            session_id,
            host,
            port,
            username,
            password,
            shared,
            options,
            input_rx,
        } = input;
        let shared_ref = Arc::clone(self);
        thread::spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_vnc_thread(RunVncThreadInput {
                    shared: Arc::clone(&shared_ref),
                    runtime: Arc::clone(&runtime),
                    session_id: session_id.clone(),
                    spec: connection::ConnectionSpec {
                        host,
                        port,
                        username,
                        password: zeroize::Zeroizing::new(password),
                        shared,
                        options,
                    },
                    input_rx,
                });
            }));
            if outcome.is_err() {
                mark_failed(MarkFailedInput {
                    shared: &shared_ref,
                    runtime: &runtime,
                    session_id: &session_id,
                    message: "VNC session stopped after an internal error".into(),
                });
            }
        });
    }

    pub(crate) fn vnc_disconnect(&self, session: &str) -> Result<()> {
        if let Some(runtime) = relock(&self.vnc_sessions).remove(session) {
            let mut rt = relock(&runtime);
            rt.input_tx = None;
            rt.status = VncSessionStatus::Disconnected;
            rt.revision = rt.revision.saturating_add(1);
        }
        let mut state = relock(&self.state);
        if let Some(rec) = state.vnc_sessions.iter_mut().find(|s| s.id == session) {
            rec.status = VncSessionStatus::Disconnected;
        }
        state.vnc_sessions.retain(|s| s.id != session);
        state.revision = state.revision.saturating_add(1);
        Ok(())
    }

    pub(crate) fn vnc_input(&self, session: &str, events: Vec<VncInputEvent>) -> Result<()> {
        let runtime = relock(&self.vnc_sessions)
            .get(session)
            .cloned()
            .context("VNC session is not live")?;
        let rt = relock(&runtime);
        let events = if rt.view_only {
            events
                .into_iter()
                .filter(|e| matches!(e, VncInputEvent::Refresh { .. }))
                .collect()
        } else {
            events
        };
        if events.is_empty() {
            return Ok(());
        }
        ensure!(
            rt.status == VncSessionStatus::Connected,
            "VNC input is disabled while disconnected or reconnecting"
        );
        input::validate(&events, &rt.options, rt.view_only)?;
        let tx = rt.input_tx.as_ref().context("VNC session is closed")?;
        ensure!(
            tx.capacity() >= events.len(),
            "VNC input queue is full; wait before sending more input"
        );
        for event in events {
            tx.try_send(event).context("VNC input queue closed")?;
        }
        Ok(())
    }

    pub(crate) fn vnc_frame(
        &self,
        session: &str,
        since: u64,
        regions: bool,
    ) -> Result<Option<VncFrame>> {
        let runtime = relock(&self.vnc_sessions)
            .get(session)
            .cloned()
            .context("VNC session is not live")?;
        let mut rt = relock(&runtime);
        // Any request counts as the viewer still watching: hidden or closed
        // views stop asking, and the refresh below stops with them.
        rt.last_poll = Some(Instant::now());
        if rt.revision <= since {
            return Ok(None);
        }
        rt.encode_frame(session, since, regions).map(Some)
    }
}
