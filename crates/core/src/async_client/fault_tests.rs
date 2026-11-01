//! Controlled transport faults: silent sockets, overlapping connections,
//! delayed replies, and owner changes during snapshot assembly.
use super::tests::{envelope, fixture, test_owner};
use super::*;
use crate::{generations::Catalog, write_frame};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;

async fn reply(socket: &mut crate::transport::AsyncStream, response: &Response) {
    let mut bytes = Vec::new();
    write_frame(&mut bytes, response).unwrap();
    socket.write_all(&bytes).await.unwrap();
}

#[tokio::test]
async fn silent_open_connection_fails_a_read_at_its_deadline() {
    let (_dir, client, listener) = fixture();
    let (release, held) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        envelope(&mut socket).await;
        // The socket stays open and never answers.
        let _ = held.await;
        drop(socket);
    });
    let request = Request::Screen {
        session: "fixture".into(),
    };
    let deadline = request.response_deadline();
    let started = Instant::now();
    let error = client.rpc(request).await.unwrap_err();
    assert!(
        matches!(error, AsyncClientError::ResponseDeadline { .. }),
        "{error:#}"
    );
    assert!(!error.is_uncertain_mutation(), "a read stays retryable");
    assert!(started.elapsed() < deadline + Duration::from_secs(2));
    let _ = release.send(());
    server.await.unwrap();
}

#[tokio::test]
async fn silent_open_connection_after_a_mutation_is_uncertain_and_sent_once() {
    let (_dir, client, listener) = fixture();
    let (release, held) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let first = envelope(&mut socket).await;
        assert!(matches!(first.request, Request::Focus { .. }));
        let _ = held.await;
        assert!(
            tokio::time::timeout(Duration::from_millis(300), listener.accept())
                .await
                .is_err(),
            "an uncertain mutation must not be replayed on a new connection"
        );
        drop(socket);
    });
    let error = client
        .rpc(Request::Focus {
            session: "fixture".into(),
        })
        .await
        .unwrap_err();
    assert!(error.is_uncertain_mutation(), "{error:#}");
    let _ = release.send(());
    server.await.unwrap();
}

#[tokio::test]
async fn concurrent_requests_receive_only_their_own_delayed_replies() {
    let (_dir, client, listener) = fixture();
    let server = tokio::spawn(async move {
        let mut held = Vec::new();
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let Request::History { session } = envelope(&mut socket).await.request else {
                panic!("expected a history read");
            };
            held.push((session, socket));
        }
        // Answer in reverse arrival order; the first reply is delayed.
        held.sort_by(|a, b| b.0.cmp(&a.0));
        for (index, (session, mut socket)) in held.into_iter().enumerate() {
            if index > 0 {
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
            reply(
                &mut socket,
                &Response::Text(format!("history of {session}")),
            )
            .await;
        }
    });
    let read = |session: &str| {
        client.rpc(Request::History {
            session: session.into(),
        })
    };
    let (a, b) = tokio::join!(read("a"), read("b"));
    assert!(matches!(a.unwrap(), Response::Text(text) if text == "history of a"));
    assert!(matches!(b.unwrap(), Response::Text(text) if text == "history of b"));
    server.await.unwrap();
}

#[tokio::test]
async fn delayed_old_snapshot_is_observed_after_the_newer_one() {
    let (_dir, client, listener) = fixture();
    let server = tokio::spawn(async move {
        let (mut old, _) = listener.accept().await.unwrap();
        envelope(&mut old).await;
        let (mut new, _) = listener.accept().await.unwrap();
        envelope(&mut new).await;
        let state = |revision| {
            Response::State(Box::new(State {
                generation: "owner".into(),
                revision,
                ..Default::default()
            }))
        };
        reply(&mut new, &state(9)).await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        reply(&mut old, &state(3)).await;
    });
    let old = tokio::spawn({
        let client = client.clone();
        async move { client.rpc(Request::Snapshot).await }
    });
    // Let the first request connect before the reconnect attempt.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let Response::State(new) = client.rpc(Request::Snapshot).await.unwrap() else {
        panic!("expected state");
    };
    let Response::State(old) = old.await.unwrap().unwrap() else {
        panic!("expected state");
    };
    assert_eq!((new.revision, old.revision), (9, 3));
    // Receipt order, not revision: the GUI must reject the late reply by
    // revision (see `delayed_reply_from_an_old_connection_*` app tests).
    assert!(old.client_observation > new.client_observation);
    server.await.unwrap();
}

#[tokio::test]
async fn owner_change_during_snapshot_assembly_supersedes_instead_of_mixing() {
    let dir = crate::tests::tempdir("supersede-");
    let root = Paths::at(dir.path().into());
    root.init().unwrap();
    generations::migrate_idle(&root).unwrap();
    let mut catalog = Catalog::open(&root).unwrap();
    let old = test_owner(&root, &catalog);
    let new = test_owner(&root, &catalog);
    catalog.set_pid(&old.id, std::process::id()).unwrap();
    catalog.activate(&old.id).unwrap();
    std::fs::write(old.paths().auth(), "fixture").unwrap();
    let listener = crate::transport::AsyncListener::bind_ipc(&old.paths().socket()).unwrap();
    let (server_root, old_id, new_id) = (root.clone(), old.id.clone(), new.id.clone());
    let server = tokio::spawn(async move {
        let mut first = true;
        while let Ok((mut socket, _)) = listener.accept().await {
            envelope(&mut socket).await;
            if first {
                // A live upgrade lands while this owner's snapshot is in flight.
                Catalog::open(&server_root)
                    .unwrap()
                    .activate(&new_id)
                    .unwrap();
                first = false;
            }
            let state = State {
                generation: old_id.clone(),
                revision: 7,
                ..Default::default()
            };
            reply(&mut socket, &Response::State(Box::new(state))).await;
        }
    });
    let client = Client::new(
        root,
        NativePool::new("catalog-supersede", 1).unwrap(),
        NativePool::new("cpu-supersede", 1).unwrap(),
    );
    let error = client.snapshot(None).await.unwrap_err();
    assert!(
        matches!(&error, AsyncClientError::Superseded { generation } if *generation == old.id),
        "{error:#}"
    );
    let Response::State(next) = client.snapshot(None).await.unwrap() else {
        panic!("expected a rebuilt state");
    };
    assert_eq!(next.generation, new.id, "the newer owner stays active");
    server.abort();
}
