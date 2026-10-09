use super::catalog::*;
use super::migration::*;
use super::ownership::*;
use super::recovery::*;
use crate::{
    Duration, Lifecycle, PROTOCOL_VERSION, Paths, Request, Response, State, atomic_write, fs, id,
};
use fs2::FileExt;
use rusqlite::Connection;

#[cfg(test)]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Envelope, Project, Session, SessionKind, now, read_frame, rpc, snapshot, write_frame,
    };

    #[test]
    fn inherited_catalog_cannot_bind_a_different_data_directory() {
        let (_directory, root, catalog) = fixture();
        let registered = owner(&root, &catalog);
        assert!(validate_endpoint(&root, &registered.paths(), &registered.id).is_ok());
        let other = tempfile::tempdir().unwrap();
        let paths = Paths::at(other.path().into());
        paths.init().unwrap();
        assert!(
            validate_endpoint(&root, &paths, &registered.id)
                .unwrap_err()
                .to_string()
                .contains("does not match")
        );
        assert!(validate_endpoint(&root, &registered.paths(), "unknown").is_err());
    }

    #[test]
    fn legacy_exit_recovery_requires_lock_and_preserves_original_records() {
        let dir = crate::tests::tempdir("legacy-");
        let paths = Paths::at(dir.path().into());
        paths.init().unwrap();
        let mut old = session("legacy-owner", Lifecycle::Running);
        // A recorded PID must never be signalled or adopted by migration.
        old.pid = Some(std::process::id());
        let state = State {
            sessions: vec![old.clone()],
            ..Default::default()
        };
        let db = Connection::open(paths.data.join("state.sqlite3")).unwrap();
        db.execute_batch("CREATE TABLE app_state(id INTEGER PRIMARY KEY,json TEXT NOT NULL); PRAGMA user_version=1;").unwrap();
        db.execute(
            "INSERT INTO app_state VALUES(1,?1)",
            [serde_json::to_string(&state).unwrap()],
        )
        .unwrap();
        let lock = fs::File::create(paths.runtime.join("daemon.lock")).unwrap();
        lock.lock_exclusive().unwrap();
        assert!(migrate_after_legacy_exit(&paths).is_err());
        assert!(!exists(&paths));
        assert!(saved(&paths).unwrap().sessions[0].lifecycle.live());
        drop(lock);
        migrate_after_legacy_exit(&paths).unwrap();
        let owners = Catalog::open(&paths).unwrap().generations().unwrap();
        assert_eq!(owners.len(), 1);
        let imported = saved(&owners[0].paths()).unwrap();
        assert_eq!(imported.sessions[0].id, old.id);
        assert_eq!(imported.sessions[0].generation, old.generation);
        assert_eq!(imported.sessions[0].lifecycle, Lifecycle::Interrupted);
        assert_eq!(imported.sessions[0].pid, None);
        assert!(saved(&paths).unwrap().sessions[0].lifecycle.live());
        let backup = Connection::open(paths.data.join("state.before-generations.sqlite3")).unwrap();
        let json: String = backup
            .query_row("SELECT json FROM app_state WHERE id=1", [], |r| r.get(0))
            .unwrap();
        assert!(
            serde_json::from_str::<State>(&json).unwrap().sessions[0]
                .lifecycle
                .live()
        );
        assert!(process_alive(std::process::id()));
    }

    #[test]
    fn discarding_a_failed_candidate_cannot_remove_an_active_owner() {
        let (_dir, paths, mut catalog) = fixture();
        let active = owner(&paths, &catalog);
        catalog.activate(&active.id).unwrap();
        let failed = owner(&paths, &catalog);
        assert!(catalog.discard_prepared(&failed.id, None).unwrap());
        assert!(!catalog.discard_prepared(&active.id, None).unwrap());
        let exited = owner(&paths, &catalog);
        catalog.set_pid(&exited.id, 123).unwrap();
        assert!(!catalog.discard_prepared(&exited.id, Some(456)).unwrap());
        assert!(catalog.discard_prepared(&exited.id, Some(123)).unwrap());
        assert_eq!(catalog.generations().unwrap().len(), 1);
        assert!(catalog.admitted(&active.id).unwrap());
    }

    fn fixture() -> (tempfile::TempDir, Paths, Catalog) {
        let dir = crate::tests::tempdir("gen-");
        let paths = Paths::at(dir.path().into());
        paths.init().unwrap();
        migrate_idle(&paths).unwrap();
        let catalog = Catalog::open(&paths).unwrap();
        (dir, paths, catalog)
    }
    fn owner(paths: &Paths, catalog: &Catalog) -> Generation {
        let id = id();
        let g = Generation {
            data: paths.data.join("generations").join(&id),
            runtime: paths.runtime.join(id.get(..8).unwrap_or(&id)),
            id,
            version: "1.0.0".into(),
            build: "fixture".into(),
            protocol: PROTOCOL_VERSION,
            catalog: CATALOG_VERSION,
            status: Status::Prepared,
            pid: None,
        };
        g.paths().init().unwrap();
        let db = Connection::open(g.data.join("state.sqlite3")).unwrap();
        db.execute_batch("CREATE TABLE app_state(id INTEGER PRIMARY KEY,json TEXT NOT NULL); PRAGMA user_version=1;").unwrap();
        db.execute(
            "INSERT INTO app_state VALUES(1,?1)",
            [serde_json::to_string(&State {
                generation: g.id.clone(),
                ..Default::default()
            })
            .unwrap()],
        )
        .unwrap();
        catalog.register(&g).unwrap();
        g
    }
    fn session(generation: &str, lifecycle: Lifecycle) -> Session {
        Session {
            id: id(),
            project_id: id(),
            label: "fixture".into(),
            cwd: "/tmp".into(),
            kind: SessionKind::Shell,
            file: None,
            lifecycle,
            created: now(),
            exit_code: None,
            rows: 24,
            cols: 80,
            generation: generation.into(),
            pid: None,
            truncated: false,
            cwd_confirmed: false,
            review: false,
        }
    }
    fn save(g: &Generation, state: &State) {
        Connection::open(g.data.join("state.sqlite3"))
            .unwrap()
            .execute(
                "UPDATE app_state SET json=?1",
                [serde_json::to_string(state).unwrap()],
            )
            .unwrap();
    }

    #[test]
    fn activation_preserves_old_session_identity_and_routes_new_work() {
        let (_dir, paths, mut catalog) = fixture();
        let a = owner(&paths, &catalog);
        let b = owner(&paths, &catalog);
        catalog.activate(&a.id).unwrap();
        let session = session(&a.id, Lifecycle::Running);
        save(
            &a,
            &State {
                generation: a.id.clone(),
                sessions: vec![session.clone()],
                ..Default::default()
            },
        );
        catalog.activate(&b.id).unwrap();
        assert!(!catalog.admitted(&a.id).unwrap());
        assert!(catalog.admitted(&b.id).unwrap());
        assert_eq!(
            owner_for(
                &paths,
                &Request::Stop {
                    session: session.id.clone()
                }
            )
            .unwrap()
            .data,
            a.data
        );
        assert_eq!(
            owner_for(
                &paths,
                &Request::AddProject {
                    path: "/tmp".into()
                }
            )
            .unwrap()
            .data,
            b.data
        );
        let preserved = saved(&a.paths()).unwrap();
        assert_eq!(preserved.sessions[0].id, session.id);
        assert!(preserved.sessions[0].lifecycle.live());
    }

    #[test]
    fn draining_owner_cannot_overwrite_shared_workspace() {
        let (_dir, paths, mut catalog) = fixture();
        let a = owner(&paths, &catalog);
        let b = owner(&paths, &catalog);
        catalog.activate(&a.id).unwrap();
        let mut state = State {
            generation: a.id.clone(),
            ..Default::default()
        };
        state.projects.push(Project {
            id: id(),
            name: "keep".into(),
            path: "/tmp".into(),
            layout: serde_json::json!({"version":999}),
        });
        catalog.save_workspace(&state).unwrap();
        catalog.activate(&b.id).unwrap();
        state.projects.clear();
        assert!(catalog.save_workspace(&state).is_err());
        catalog.refresh(&mut state).unwrap();
        assert_eq!(state.projects[0].layout["version"], 999);
    }

    #[test]
    fn frozen_activation_rolls_back_all_generation_statuses() {
        let (_dir, paths, mut catalog) = fixture();
        let a = owner(&paths, &catalog);
        let b = owner(&paths, &catalog);
        catalog.activate(&a.id).unwrap();
        catalog.freeze(true).unwrap();
        assert!(!catalog.admitted(&a.id).unwrap());
        assert!(catalog.activate(&b.id).is_err());
        assert_eq!(catalog.active().unwrap().as_deref(), Some(a.id.as_str()));
        assert_eq!(catalog.generations().unwrap()[0].status, Status::Active);
        catalog.freeze(false).unwrap();
        assert!(catalog.admitted(&a.id).unwrap());
    }

    #[test]
    fn unavailable_owner_keeps_live_ownership_and_blocks_death_inference() {
        let (_dir, paths, mut catalog) = fixture();
        let a = owner(&paths, &catalog);
        catalog.set_pid(&a.id, std::process::id()).unwrap();
        catalog.activate(&a.id).unwrap();
        save(
            &a,
            &State {
                generation: a.id.clone(),
                sessions: vec![session(&a.id, Lifecycle::Running)],
                ..Default::default()
            },
        );
        let lock = fs::File::create(a.runtime.join("daemon.lock")).unwrap();
        lock.lock_exclusive().unwrap();
        let registered = catalog.generations().unwrap().remove(0);
        assert!(!recover_exited(&paths, &registered).unwrap());
        let inventory = snapshot(&paths).unwrap();
        assert!(inventory.sessions[0].lifecycle.live());
        assert!(inventory.generations[0].error.is_some());
        drop(lock);
        assert!(
            !recover_exited(&paths, &registered).unwrap(),
            "A live process still owns the identity even without the lock"
        );
    }

    #[test]
    fn empty_catalog_file_does_not_block_migration() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path().into());
        paths.init().unwrap();
        fs::write(paths.data.join("catalog.sqlite3"), []).unwrap();
        assert!(!exists(&paths));
        migrate_idle(&paths).unwrap();
        assert_eq!(catalog_version(&paths), Some(CATALOG_VERSION));
        Catalog::open(&paths).unwrap();
    }

    #[test]
    fn migration_rejects_live_sessions_without_publishing_catalog() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path().into());
        paths.init().unwrap();
        let legacy = State {
            sessions: vec![session("legacy", Lifecycle::Running)],
            ..Default::default()
        };
        assert!(Catalog::import(&paths, &legacy).is_err());
        assert!(!exists(&paths));
    }

    #[test]
    fn migration_keeps_session_ids_scrollback_and_original_database() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path().into());
        paths.init().unwrap();
        let session = session("legacy", Lifecycle::Ended);
        let state = State {
            sessions: vec![session.clone()],
            ..Default::default()
        };
        let db = Connection::open(paths.data.join("state.sqlite3")).unwrap();
        db.execute_batch("CREATE TABLE app_state(id INTEGER PRIMARY KEY,json TEXT NOT NULL); PRAGMA user_version=1;").unwrap();
        db.execute(
            "INSERT INTO app_state VALUES(1,?1)",
            [serde_json::to_string(&state).unwrap()],
        )
        .unwrap();
        let history = format!("{}-00000000000000000001.pty", session.id);
        fs::write(paths.history_dir().join(&history), b"retained").unwrap();
        migrate_idle(&paths).unwrap();
        let archive = Catalog::open(&paths)
            .unwrap()
            .generations()
            .unwrap()
            .remove(0);
        assert_eq!(saved(&archive.paths()).unwrap().sessions[0].id, session.id);
        assert_eq!(
            fs::read(archive.paths().history_dir().join(&history)).unwrap(),
            b"retained"
        );
        assert_eq!(
            fs::read(paths.history_dir().join(&history)).unwrap(),
            b"retained"
        );
        assert!(
            paths
                .data
                .join("state.before-generations.sqlite3")
                .is_file()
        );
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            2
        );
    }

    #[test]
    fn coordination_serializes_activation_against_admission() {
        let (_dir, paths, mut catalog) = fixture();
        let a = owner(&paths, &catalog);
        let b = owner(&paths, &catalog);
        catalog.activate(&a.id).unwrap();
        let guard = coordinate(&paths).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let other = paths.clone();
        let b_id = b.id.clone();
        let worker = std::thread::spawn(move || {
            let _guard = coordinate(&other).unwrap();
            Catalog::open(&other).unwrap().activate(&b_id).unwrap();
            tx.send(()).unwrap();
        });
        assert!(rx.recv_timeout(Duration::from_millis(50)).is_err());
        assert!(catalog.admitted(&a.id).unwrap());
        drop(guard);
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.join().unwrap();
        assert!(!catalog.admitted(&a.id).unwrap());
    }

    #[test]
    fn uncertain_creation_response_is_never_retried() {
        let (_dir, paths, mut catalog) = fixture();
        let a = owner(&paths, &catalog);
        catalog.activate(&a.id).unwrap();
        atomic_write(&a.paths().auth(), b"fixture").unwrap();
        let listener = crate::transport::Listener::bind_ipc(&a.paths().socket()).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let envelope: Envelope = read_frame(&mut stream).unwrap();
            assert!(matches!(envelope.request, Request::Create { .. }));
            // Model a spawn whose response is lost. The client cannot infer that
            // execution failed, and must never send a second creation.
            drop(stream);
            listener
        });
        let request = Request::Create {
            project: id(),
            cwd: None,
            file: None,
            line: None,
            column: None,
            editor: false,
        };
        assert!(rpc(&paths, request).is_err());
        let listener = server.join().unwrap();
        listener.set_nonblocking(true).unwrap();
        assert!(listener.accept().is_err());
    }

    #[test]
    fn only_explicit_pre_spawn_redirect_retries_on_active_owner() {
        let (_dir, paths, mut catalog) = fixture();
        let a = owner(&paths, &catalog);
        let b = owner(&paths, &catalog);
        catalog.activate(&a.id).unwrap();
        for g in [&a, &b] {
            atomic_write(&g.paths().auth(), b"fixture").unwrap();
        }
        let first = crate::transport::Listener::bind_ipc(&a.paths().socket()).unwrap();
        let second = crate::transport::Listener::bind_ipc(&b.paths().socket()).unwrap();
        let root = paths.clone();
        let new_id = b.id.clone();
        let created = session(&b.id, Lifecycle::Running);
        let expected = created.id.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = first.accept().unwrap();
            let _: Envelope = read_frame(&mut stream).unwrap();
            {
                let _guard = coordinate(&root).unwrap();
                Catalog::open(&root).unwrap().activate(&new_id).unwrap();
            }
            write_frame(&mut stream, &Response::Redirect { generation: new_id }).unwrap();
            let (mut stream, _) = second.accept().unwrap();
            let _: Envelope = read_frame(&mut stream).unwrap();
            write_frame(&mut stream, &Response::Created(created)).unwrap();
        });
        let Response::Created(actual) = rpc(
            &paths,
            Request::Create {
                project: id(),
                cwd: None,
                file: None,
                line: None,
                column: None,
                editor: false,
            },
        )
        .unwrap() else {
            panic!("created");
        };
        assert_eq!(actual.id, expected);
        assert_eq!(actual.generation, b.id);
        server.join().unwrap();
    }

    #[test]
    fn interrupted_import_keeps_backup_and_retries_without_duplicate_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path().into());
        paths.init().unwrap();
        let historical = session("legacy", Lifecycle::Ended);
        let state = State {
            sessions: vec![historical.clone()],
            ..Default::default()
        };
        let db = Connection::open(paths.data.join("state.sqlite3")).unwrap();
        db.execute_batch("CREATE TABLE app_state(id INTEGER PRIMARY KEY,json TEXT NOT NULL); PRAGMA user_version=1;").unwrap();
        db.execute(
            "INSERT INTO app_state VALUES(1,?1)",
            [serde_json::to_string(&state).unwrap()],
        )
        .unwrap();
        fs::remove_dir(paths.history_dir()).unwrap();
        assert!(migrate_idle(&paths).is_err());
        assert!(!exists(&paths));
        assert!(
            paths
                .data
                .join("state.before-generations.sqlite3")
                .is_file()
        );
        assert_eq!(saved(&paths).unwrap().sessions[0].id, historical.id);
        fs::create_dir(paths.history_dir()).unwrap();
        migrate_idle(&paths).unwrap();
        let owners = Catalog::open(&paths).unwrap().generations().unwrap();
        assert_eq!(owners.len(), 1);
        assert_eq!(
            saved(&owners[0].paths()).unwrap().sessions[0].id,
            historical.id
        );
    }

    #[test]
    fn incompatible_candidate_and_second_recovery_preserve_current_owner() {
        let (_dir, paths, mut catalog) = fixture();
        let a = owner(&paths, &catalog);
        catalog.activate(&a.id).unwrap();
        let mut incompatible = a.clone();
        incompatible.id = id();
        incompatible.protocol += 1;
        assert!(catalog.register(&incompatible).is_err());
        assert_eq!(catalog.active().unwrap().as_deref(), Some(a.id.as_str()));
        catalog.freeze(true).unwrap();
        assert!(catalog.freeze(true).is_err());
        catalog.freeze(false).unwrap();
        assert!(catalog.admitted(&a.id).unwrap());
    }

    #[test]
    fn restore_serving_repairs_catalog_active_marked_retired() {
        let (_dir, paths, mut catalog) = fixture();
        let a = owner(&paths, &catalog);
        catalog.activate(&a.id).unwrap();
        catalog.retire(&a.id).unwrap();
        assert_eq!(catalog.generations().unwrap()[0].status, Status::Retired);
        assert_eq!(catalog.active().unwrap().as_deref(), Some(a.id.as_str()));
        assert!(catalog.restore_serving().unwrap());
        assert_eq!(catalog.generations().unwrap()[0].status, Status::Active);
        assert!(catalog.admitted(&a.id).unwrap());
        assert!(!catalog.restore_serving().unwrap());
    }

    #[test]
    fn create_reaches_catalog_active_owner_marked_retired() {
        let (_dir, paths, mut catalog) = fixture();
        let a = owner(&paths, &catalog);
        catalog.activate(&a.id).unwrap();
        catalog.retire(&a.id).unwrap();
        atomic_write(&a.paths().auth(), b"fixture").unwrap();
        let listener = crate::transport::Listener::bind_ipc(&a.paths().socket()).unwrap();
        let generation = a.id.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let envelope: Envelope = read_frame(&mut stream).unwrap();
            assert!(
                matches!(envelope.request, Request::Create { .. }),
                "Create must not be wrapped as Archived"
            );
            write_frame(
                &mut stream,
                &Response::Created(session(&generation, Lifecycle::Running)),
            )
            .unwrap();
        });
        let Response::Created(_) = rpc(
            &paths,
            Request::Create {
                project: id(),
                cwd: None,
                file: None,
                line: None,
                column: None,
                editor: false,
            },
        )
        .unwrap() else {
            panic!("created");
        };
        server.join().unwrap();
    }

    #[test]
    fn historical_owner_requests_stay_archived() {
        let (_dir, paths, mut catalog) = fixture();
        let a = owner(&paths, &catalog);
        let b = owner(&paths, &catalog);
        catalog.activate(&a.id).unwrap();
        let record = session(&a.id, Lifecycle::Ended);
        save(
            &a,
            &State {
                generation: a.id.clone(),
                sessions: vec![record.clone()],
                ..Default::default()
            },
        );
        catalog.activate(&b.id).unwrap();
        catalog.retire(&a.id).unwrap();
        atomic_write(&b.paths().auth(), b"fixture").unwrap();
        let listener = crate::transport::Listener::bind_ipc(&b.paths().socket()).unwrap();
        let expected = record.id.clone();
        let archived = a.id.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let envelope: Envelope = read_frame(&mut stream).unwrap();
            match envelope.request {
                Request::Archived {
                    generation,
                    request,
                } => {
                    assert_eq!(generation, archived);
                    assert!(matches!(
                        *request,
                        Request::History { session } if session == expected
                    ));
                }
                other => panic!("{other:?}"),
            }
            write_frame(&mut stream, &Response::Ok).unwrap();
        });
        rpc(
            &paths,
            Request::History {
                session: record.id.clone(),
            },
        )
        .unwrap();
        server.join().unwrap();
    }

    #[test]
    fn snapshot_contacts_catalog_active_owner_marked_retired() {
        let (_dir, paths, mut catalog) = fixture();
        let a = owner(&paths, &catalog);
        catalog.activate(&a.id).unwrap();
        save(
            &a,
            &State {
                generation: a.id.clone(),
                daemon_version: Some("from-disk".into()),
                ..Default::default()
            },
        );
        catalog.retire(&a.id).unwrap();
        atomic_write(&a.paths().auth(), b"fixture").unwrap();
        let listener = crate::transport::Listener::bind_ipc(&a.paths().socket()).unwrap();
        let generation = a.id.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let envelope: Envelope = read_frame(&mut stream).unwrap();
            assert!(matches!(envelope.request, Request::Snapshot));
            snapshot::write_response(
                &mut stream,
                &Response::State(Box::new(State {
                    generation,
                    daemon_version: Some("from-socket".into()),
                    ..Default::default()
                })),
                envelope.snapshot_chunks,
            )
            .unwrap();
        });
        let state = snapshot(&paths).unwrap();
        assert_eq!(state.daemon_version.as_deref(), Some("from-socket"));
        server.join().unwrap();
    }

    #[test]
    fn recover_exited_does_not_retire_a_listening_catalog_active_owner() {
        let (_dir, paths, mut catalog) = fixture();
        let a = owner(&paths, &catalog);
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        catalog.set_pid(&a.id, pid).unwrap();
        catalog.activate(&a.id).unwrap();
        save(
            &a,
            &State {
                generation: a.id.clone(),
                sessions: vec![session(&a.id, Lifecycle::Running)],
                ..Default::default()
            },
        );
        let _listener = crate::transport::Listener::bind_ipc(&a.paths().socket()).unwrap();
        let registered = catalog.generations().unwrap().remove(0);
        assert!(!recover_exited(&paths, &registered).unwrap());
        assert_eq!(
            Catalog::open(&paths).unwrap().generations().unwrap()[0].status,
            Status::Active
        );
        assert!(saved(&a.paths()).unwrap().sessions[0].lifecycle.live());
    }

    #[test]
    fn missing_runtime_interrupts_draining_sessions_despite_pid_reuse() {
        let (_dir, paths, mut catalog) = fixture();
        let drained = owner(&paths, &catalog);
        let active = owner(&paths, &catalog);
        catalog.set_pid(&drained.id, std::process::id()).unwrap();
        catalog.activate(&drained.id).unwrap();
        catalog.activate(&active.id).unwrap();
        save(
            &drained,
            &State {
                generation: drained.id.clone(),
                sessions: vec![session(&drained.id, Lifecycle::Running)],
                ..Default::default()
            },
        );
        fs::remove_dir_all(&drained.runtime).unwrap();
        assert!(process_alive(std::process::id()));
        let registered = catalog
            .generations()
            .unwrap()
            .into_iter()
            .find(|generation| generation.id == drained.id)
            .unwrap();
        assert!(recover_exited(&paths, &registered).unwrap());
        let recovered = saved(&registered.paths()).unwrap();
        assert_eq!(recovered.sessions[0].lifecycle, Lifecycle::Interrupted);
        assert!(recovered.sessions[0].pid.is_none());
        assert_eq!(
            Catalog::open(&paths)
                .unwrap()
                .generations()
                .unwrap()
                .into_iter()
                .find(|generation| generation.id == drained.id)
                .unwrap()
                .status,
            Status::Retired
        );
        assert!(
            Catalog::open(&paths)
                .unwrap()
                .generations()
                .unwrap()
                .into_iter()
                .any(|generation| {
                    generation.id == active.id && generation.status == Status::Active
                })
        );
    }

    fn presence_generation(status: Status) -> Generation {
        Generation {
            id: "owner".into(),
            data: "/tmp/owner".into(),
            runtime: "/tmp/owner-run".into(),
            version: "1.0.0".into(),
            build: "fixture".into(),
            protocol: PROTOCOL_VERSION,
            catalog: CATALOG_VERSION,
            status,
            pid: None,
        }
    }

    #[test]
    fn presence_merges_only_from_live_capable_owners() {
        let capable = vec![crate::AGENT_PRESENCE_CAPABILITY.into()];
        // Live owner advertising the capability merges.
        assert!(mergeable_presence(
            &presence_generation(Status::Active),
            &capable,
            &None,
            "active",
        ));
        assert!(mergeable_presence(
            &presence_generation(Status::Draining),
            &capable,
            &None,
            "active",
        ));
        // Historical fallback never merges, even when capable.
        assert!(!mergeable_presence(
            &presence_generation(Status::Retired),
            &capable,
            &None,
            "active",
        ));
        // Unavailable owners never merge.
        assert!(!mergeable_presence(
            &presence_generation(Status::Active),
            &capable,
            &Some("Owner unavailable".into()),
            "active",
        ));
        // Older daemons without the capability never merge.
        assert!(!mergeable_presence(
            &presence_generation(Status::Active),
            &[],
            &None,
            "active",
        ));
    }

    #[test]
    fn clear_owned_and_saved_states_carry_no_presence() {
        let mut state = State {
            presence: vec![crate::agents::TerminalPresence {
                session_id: "s".into(),
                generation: "g".into(),
                agents: Vec::new(),
                outcome: crate::agents::PresenceOutcome::Verified,
                observed_at: now(),
            }],
            ..Default::default()
        };
        clear_owned(&mut state);
        assert!(state.presence.is_empty());
        let (_dir, paths, catalog) = fixture();
        let g = owner(&paths, &catalog);
        let mut stored = State {
            generation: g.id.clone(),
            presence: vec![crate::agents::TerminalPresence {
                session_id: "s".into(),
                generation: g.id.clone(),
                agents: Vec::new(),
                outcome: crate::agents::PresenceOutcome::Verified,
                observed_at: now(),
            }],
            ..Default::default()
        };
        save(&g, &stored);
        assert!(saved(&g.paths()).unwrap().presence.is_empty());
        stored.presence.clear();
        save(&g, &stored);
    }

    #[test]
    fn mixed_snapshot_excludes_historical_and_unavailable_presence() {
        // No sockets: the live owner is unreachable, so it falls back to its
        // saved state with an availability error. Neither it nor the retired
        // owner may contribute presence to the aggregate.
        let (_dir, paths, mut catalog) = fixture();
        let retired = owner(&paths, &catalog);
        let active = owner(&paths, &catalog);
        catalog.activate(&retired.id).unwrap();
        catalog.activate(&active.id).unwrap();
        catalog.retire(&retired.id).unwrap();
        let observed = |generation: &str| crate::agents::TerminalPresence {
            session_id: format!("session-{generation}"),
            generation: generation.into(),
            agents: vec![crate::agents::DetectedAgent {
                kind: "codex".into(),
                process: crate::agents::ProcessIdentity {
                    pid: 42,
                    start_time: 42,
                },
                foreground: true,
            }],
            outcome: crate::agents::PresenceOutcome::Verified,
            observed_at: now(),
        };
        save(
            &retired,
            &State {
                generation: retired.id.clone(),
                sessions: vec![session(&retired.id, Lifecycle::Ended)],
                capabilities: vec![crate::AGENT_PRESENCE_CAPABILITY.into()],
                presence: vec![observed(&retired.id)],
                ..Default::default()
            },
        );
        save(
            &active,
            &State {
                generation: active.id.clone(),
                sessions: vec![session(&active.id, Lifecycle::Running)],
                capabilities: vec![crate::AGENT_PRESENCE_CAPABILITY.into()],
                presence: vec![observed(&active.id)],
                ..Default::default()
            },
        );
        let aggregate = snapshot(&paths).unwrap();
        assert!(aggregate.presence.is_empty());
        // Sessions and hook records still merge; only presence is excluded.
        assert_eq!(aggregate.sessions.len(), 2);
        let health = aggregate
            .generations
            .iter()
            .find(|g| g.owner.id == active.id)
            .unwrap();
        assert!(health.error.is_some());
        assert!(
            health
                .capabilities
                .contains(&crate::AGENT_PRESENCE_CAPABILITY.to_string())
        );
    }

    #[test]
    fn retired_generations_are_pruned_beyond_the_retention_limit() {
        let (_dir, paths, catalog) = fixture();
        let mut ids = Vec::new();
        for _ in 0..6 {
            let g = owner(&paths, &catalog);
            catalog.retire(&g.id).unwrap();
            ids.push(g.id);
        }
        assert_eq!(prune_retired(&paths, 4).unwrap(), 2);
        let remaining: Vec<String> = Catalog::open(&paths)
            .unwrap()
            .generations()
            .unwrap()
            .into_iter()
            .filter(|g| g.status == Status::Retired)
            .map(|g| g.id)
            .collect();
        assert_eq!(remaining.len(), 4);
        assert!(!remaining.contains(&ids[0]));
        assert!(!remaining.contains(&ids[1]));
        assert!(remaining.contains(&ids[5]));
        assert!(!paths.data.join("generations").join(&ids[0]).exists());
        assert!(paths.data.join("generations").join(&ids[5]).exists());
        // Nothing left to prune on a second pass.
        assert_eq!(prune_retired(&paths, 4).unwrap(), 0);
    }

    #[test]
    fn retired_generation_with_live_records_is_never_pruned() {
        let (_dir, paths, catalog) = fixture();
        let live = owner(&paths, &catalog);
        save(
            &live,
            &State {
                generation: live.id.clone(),
                sessions: vec![session(&live.id, Lifecycle::Running)],
                ..Default::default()
            },
        );
        catalog.retire(&live.id).unwrap();
        for _ in 0..4 {
            let g = owner(&paths, &catalog);
            catalog.retire(&g.id).unwrap();
        }
        assert_eq!(prune_retired(&paths, 1).unwrap(), 3);
        let owners = Catalog::open(&paths).unwrap().generations().unwrap();
        assert!(owners.iter().any(|g| g.id == live.id));
        assert!(live.data.exists());
    }
}
