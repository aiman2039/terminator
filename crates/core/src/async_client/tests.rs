use super::*;
use crate::{Envelope, MAX_FRAME, PROTOCOL_VERSION, transport::AsyncStream, write_frame};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
fn fixture() -> (tempfile::TempDir, Client, crate::transport::AsyncListener) {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::at(dir.path().into());
    paths.init().unwrap();
    std::fs::write(paths.auth(), "fixture").unwrap();
    let listener = crate::transport::AsyncListener::bind_ipc(&paths.socket()).unwrap();
    let client = Client::new(
        paths,
        NativePool::new("catalog-test", 1).unwrap(),
        NativePool::new("cpu-test", 2).unwrap(),
    );
    (dir, client, listener)
}
async fn envelope(socket: &mut AsyncStream) -> Envelope {
    let len = socket.read_u32().await.unwrap() as usize;
    let mut bytes = vec![0; len];
    socket.read_exact(&mut bytes).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
#[tokio::test]
async fn large_vnc_frame_uses_opt_in_chunks_over_async_transport() {
    let (_dir, client, listener) = fixture();
    let expected = crate::vnc::VncFrame {
        session: "fixture".into(),
        width: 2048,
        height: 1536,
        revision: 7,
        status: crate::vnc::VncSessionStatus::Connected,
        error: None,
        zlib_rgba: "A".repeat(MAX_FRAME),
        region: None,
        clipboard: None,
        cursor: None,
        monitors: Vec::new(),
        resize_supported: false,
    };
    let frame = expected.clone();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = envelope(&mut socket).await;
        assert!(request.snapshot_chunks);
        assert!(matches!(request.request, Request::VncFrame { .. }));
        let mut wire = Vec::new();
        crate::snapshot::write_response(
            &mut wire,
            &Response::VncFrame(Box::new(frame)),
            request.snapshot_chunks,
        )
        .unwrap();
        socket.write_all(&wire).await.unwrap();
    });
    let Response::VncFrame(actual) = client
        .rpc(Request::VncFrame {
            session: "fixture".into(),
            since: 0,
            regions: true,
        })
        .await
        .unwrap()
    else {
        panic!("Expected VNC frame")
    };
    assert_eq!(*actual, expected);
    server.await.unwrap();
}

#[tokio::test]
async fn fragmented_legacy_response_preserves_snapshot_hint_and_auth() {
    let (_dir, client, listener) = fixture();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = envelope(&mut socket).await;
        assert_eq!(request.auth, "fixture");
        assert!(request.snapshot_chunks);
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &Response::State(Box::default())).unwrap();
        for fragment in bytes.chunks(3) {
            socket.write_all(fragment).await.unwrap();
            tokio::task::yield_now().await;
        }
    });
    assert!(matches!(
        client.rpc(Request::Snapshot).await.unwrap(),
        Response::State(_)
    ));
    server.await.unwrap();
}
#[tokio::test]
async fn connection_loss_after_mutation_is_uncertain_and_never_replayed() {
    let (_dir, client, listener) = fixture();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        envelope(&mut socket).await;
        drop(socket);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
                .await
                .is_err()
        );
    });
    let error = client
        .rpc(Request::Focus {
            session: "fixture".into(),
        })
        .await
        .unwrap_err();
    assert!(error.is_uncertain_mutation(), "{error:#}");
    server.await.unwrap();
}
#[tokio::test]
async fn archived_history_read_loss_is_certain_and_retryable() {
    let (_dir, client, listener) = fixture();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        envelope(&mut socket).await;
        drop(socket);
    });
    let error = client
        .rpc(Request::Archived {
            generation: "retired".into(),
            request: Box::new(Request::History {
                session: "s".into(),
            }),
        })
        .await
        .unwrap_err();
    assert!(
        !error.is_uncertain_mutation(),
        "a retired-session read must stay retryable: {error:#}"
    );
    server.await.unwrap();
}
#[tokio::test]
async fn oversized_response_is_rejected_before_allocation() {
    let (_dir, client, listener) = fixture();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        envelope(&mut socket).await;
        socket
            .write_u32(u32::try_from(MAX_FRAME.saturating_add(1)).unwrap_or(u32::MAX))
            .await
            .unwrap();
    });
    assert!(
        client
            .rpc(Request::Snapshot)
            .await
            .unwrap_err()
            .to_string()
            .contains("too large")
    );
    server.await.unwrap();
}
#[tokio::test]
async fn catalog_snapshot_short_circuits_unchanged_without_rebuilding() {
    use crate::generations::{self, Catalog, Generation, Status};
    let dir = crate::tests::tempdir("snap-");
    let root = Paths::at(dir.path().into());
    root.init().unwrap();
    generations::migrate_idle(&root).unwrap();
    let id = crate::id();
    let owner = Generation {
        id: id.clone(),
        data: root.data.join("generations").join(&id),
        runtime: root.runtime.join(id.get(..8).unwrap_or(&id)),
        version: "test".into(),
        build: "test".into(),
        protocol: PROTOCOL_VERSION,
        catalog: generations::CATALOG_VERSION,
        status: Status::Prepared,
        pid: None,
    };
    owner.paths().init().unwrap();
    let db = rusqlite::Connection::open(owner.data.join("state.sqlite3")).unwrap();
    db.execute_batch(
        "CREATE TABLE app_state(id INTEGER PRIMARY KEY,json TEXT NOT NULL); PRAGMA user_version=1;",
    )
    .unwrap();
    db.execute(
        "INSERT INTO app_state VALUES(1,?1)",
        [serde_json::to_string(&State {
            generation: id.clone(),
            ..Default::default()
        })
        .unwrap()],
    )
    .unwrap();
    let mut catalog = Catalog::open(&root).unwrap();
    catalog.register(&owner).unwrap();
    catalog.set_pid(&id, std::process::id()).unwrap();
    catalog.activate(&id).unwrap();
    std::fs::write(owner.paths().auth(), "fixture").unwrap();
    let served = State {
        generation: id.clone(),
        revision: 7,
        ..Default::default()
    };
    let listener = crate::transport::AsyncListener::bind_ipc(&owner.paths().socket()).unwrap();
    let server = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let _ = envelope(&mut socket).await;
            let mut bytes = Vec::new();
            write_frame(&mut bytes, &Response::State(Box::new(served.clone()))).unwrap();
            let _ = socket.write_all(&bytes).await;
        }
    });
    let client = Client::new(
        root,
        NativePool::new("catalog-snap", 1).unwrap(),
        NativePool::new("cpu-snap", 1).unwrap(),
    );
    let Response::State(first) = client.snapshot(None).await.unwrap() else {
        panic!("expected a built state");
    };
    assert_eq!(first.revision, 7);
    assert!(matches!(
        client.snapshot(Some(first.snapshot_hint())).await.unwrap(),
        Response::Unchanged
    ));
    server.abort();
}

#[test]
fn starting_an_ssh_transfer_is_a_mutation_but_polling_it_is_not() {
    let request = |operation| Request::RemoteProject {
        project: "p".into(),
        operation,
    };
    assert!(!read_only(&request(crate::remote::Operation::Upload {
        local: "/tmp/a".into(),
        remote_dir: String::new(),
    })));
    assert!(!read_only(&request(crate::remote::Operation::Download {
        path: "a".into(),
        local: "/tmp/a".into(),
    })));
    assert!(read_only(&request(
        crate::remote::Operation::TransferStatus { id: "x".into() }
    )));
    assert!(read_only(&request(crate::remote::Operation::List {
        path: String::new()
    })));
}

#[test]
fn archived_mutation_marks_only_retired_writes_for_cache_eviction() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::at(dir.path().into());
    paths.init().unwrap();
    let client = Client::new(
        paths,
        NativePool::new("evict-test", 1).unwrap(),
        NativePool::new("evict-cpu-test", 1).unwrap(),
    );
    client
        .historical
        .lock()
        .unwrap()
        .insert("retired".to_string(), std::sync::Arc::new(State::default()));
    // Plain (non-archived) requests never touch the retired cache.
    assert!(
        archived_mutation(&Request::Notice {
            id: "n".into(),
            action: "dismiss".into(),
        })
        .is_none()
    );
    // Archived history reads are served from disk without changing it.
    assert!(
        archived_mutation(&Request::Archived {
            generation: "retired".into(),
            request: Box::new(Request::History {
                session: "s".into()
            }),
        })
        .is_none()
    );
    assert_eq!(
        archived_mutation(&Request::Archived {
            generation: "retired".into(),
            request: Box::new(Request::Notice {
                id: "n".into(),
                action: "dismiss".into(),
            }),
        }),
        Some("retired")
    );
    client.evict_historical("retired");
    assert!(
        client.historical.lock().unwrap().is_empty(),
        "eviction must drop the touched retired snapshot"
    );
}

#[tokio::test]
async fn archived_notice_dismissal_is_visible_to_the_next_snapshot() {
    use crate::generations::{self, Catalog, Generation, Status};
    let dir = crate::tests::tempdir("archived-notice-");
    let root = Paths::at(dir.path().into());
    root.init().unwrap();
    generations::migrate_idle(&root).unwrap();
    fn test_owner(root: &Paths, catalog: &Catalog) -> Generation {
        let id = crate::id();
        let owner = Generation {
            data: root.data.join("generations").join(&id),
            runtime: root.runtime.join(id.get(..8).unwrap_or(&id)),
            id,
            version: "1.0.0".into(),
            build: "fixture".into(),
            protocol: PROTOCOL_VERSION,
            catalog: generations::CATALOG_VERSION,
            status: Status::Prepared,
            pid: None,
        };
        owner.paths().init().unwrap();
        let db = rusqlite::Connection::open(owner.data.join("state.sqlite3")).unwrap();
        db.execute_batch(
            "CREATE TABLE app_state(id INTEGER PRIMARY KEY,json TEXT NOT NULL); PRAGMA user_version=1;",
        )
        .unwrap();
        db.execute(
            "INSERT INTO app_state VALUES(1,?1)",
            [serde_json::to_string(&State {
                generation: owner.id.clone(),
                ..Default::default()
            })
            .unwrap()],
        )
        .unwrap();
        catalog.register(&owner).unwrap();
        owner
    }
    fn save_state(owner: &Generation, state: &State) {
        rusqlite::Connection::open(owner.data.join("state.sqlite3"))
            .unwrap()
            .execute(
                "UPDATE app_state SET json=?1",
                [serde_json::to_string(state).unwrap()],
            )
            .unwrap();
    }
    let mut catalog = Catalog::open(&root).unwrap();
    let retired = test_owner(&root, &catalog);
    let active = test_owner(&root, &catalog);
    let notice_id = crate::id();
    let stale_session = crate::Session {
        remote: None,
        id: crate::id(),
        project_id: crate::id(),
        label: "old terminal".into(),
        cwd: "/tmp".into(),
        kind: crate::SessionKind::Shell,
        file: None,
        lifecycle: crate::Lifecycle::Interrupted,
        created: crate::now(),
        exit_code: None,
        rows: 24,
        cols: 80,
        generation: retired.id.clone(),
        pid: None,
        truncated: false,
        cwd_confirmed: false,
        review: false,
    };
    save_state(
        &retired,
        &State {
            generation: retired.id.clone(),
            revision: 1,
            sessions: vec![stale_session.clone()],
            notifications: vec![crate::Notification {
                id: notice_id.clone(),
                session_id: stale_session.id.clone(),
                invocation_id: crate::id(),
                request_id: None,
                state: crate::AgentState::WaitingInput,
                summary: "waiting".into(),
                details: String::new(),
                created: crate::now(),
                read: false,
                dismissed: false,
                resolved: false,
                snoozed_until: 0,
            }],
            ..Default::default()
        },
    );
    catalog.activate(&active.id).unwrap();
    catalog.retire(&retired.id).unwrap();
    std::fs::write(active.paths().auth(), "fixture").unwrap();
    let served = State {
        generation: active.id.clone(),
        revision: 1,
        ..Default::default()
    };
    let retired_id = retired.id.clone();
    let evicted_id = retired_id.clone();
    let expected_notice = notice_id.clone();
    let listener = crate::transport::AsyncListener::bind_ipc(&active.paths().socket()).unwrap();
    let server = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let request = envelope(&mut socket).await;
            match request.request {
                Request::Snapshot => {
                    let mut bytes = Vec::new();
                    crate::snapshot::write_response(
                        &mut bytes,
                        &Response::State(Box::new(served.clone())),
                        request.snapshot_chunks,
                    )
                    .unwrap();
                    if socket.write_all(&bytes).await.is_err() {
                        break;
                    }
                }
                Request::Archived {
                    generation,
                    request,
                } if generation == retired_id => {
                    let Request::Notice { id, action } = *request else {
                        panic!("expected an archived notice action");
                    };
                    assert_eq!(id, expected_notice);
                    assert_eq!(action, "dismiss");
                    // Mimic the daemon's archived handler: the retired
                    // owner's disk state carries the dismissal.
                    let mut state = generations::saved(&retired.paths()).unwrap();
                    let notice = state
                        .notifications
                        .iter_mut()
                        .find(|notice| notice.id == id)
                        .unwrap();
                    notice.dismissed = true;
                    state.revision += 1;
                    save_state(&retired, &state);
                    let mut bytes = Vec::new();
                    write_frame(&mut bytes, &Response::Ok).unwrap();
                    if socket.write_all(&bytes).await.is_err() {
                        break;
                    }
                }
                other => panic!("unexpected request: {other:?}"),
            }
        }
    });
    let client = Client::new(
        root,
        NativePool::new("catalog-dismiss", 1).unwrap(),
        NativePool::new("cpu-dismiss", 1).unwrap(),
    );
    let Response::State(first) = client.snapshot(None).await.unwrap() else {
        panic!("expected a built state");
    };
    assert!(
        first
            .notifications
            .iter()
            .any(|notice| notice.id == notice_id && !notice.dismissed),
        "the retired notice is pending before dismissal"
    );
    client
        .rpc(Request::Notice {
            id: notice_id.clone(),
            action: "dismiss".into(),
        })
        .await
        .unwrap();
    assert!(
        !client.historical.lock().unwrap().contains_key(&evicted_id),
        "the archived dismissal must drop the cached retired snapshot"
    );
    let Response::State(second) = client.snapshot(Some(first.snapshot_hint())).await.unwrap()
    else {
        panic!("the dismissal must change the next snapshot");
    };
    assert!(
        second
            .notifications
            .iter()
            .find(|notice| notice.id == notice_id)
            .is_some_and(|notice| notice.dismissed),
        "the dismissed retired notice must stay gone"
    );
    server.abort();
}
