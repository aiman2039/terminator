//! Shared-memory frames: what the daemon publishes and what a viewer reads.
use super::shm::PublishInput;
use super::*;
use terminator_core::vnc::VncShmRequest;
use terminator_sys::{FrameRect, SharedFrameReader};

fn runtime() -> VncRuntime {
    let (input_tx, _input_rx) = mpsc::channel(256);
    VncRuntime::new(false, input_tx)
}

fn publish(rt: &mut VncRuntime, since: u64, dir: &std::path::Path, cursor: u64) -> VncFrame {
    rt.publish_shm(PublishInput {
        session: "s-1",
        since,
        request: VncShmRequest {
            wait_ms: 0,
            cursor_serial: cursor,
        },
        runtime_dir: dir,
    })
    .unwrap()
}

fn blit(rt: &mut VncRuntime, x: u16, y: u16, rgba: [u8; 4]) {
    rt.blit_rgba(
        Rect {
            x,
            y,
            width: 1,
            height: 1,
        },
        &rgba,
    );
}

#[test]
fn a_viewer_at_the_published_revision_gets_each_changed_rectangle() {
    let dir = tempfile::tempdir().unwrap();
    let mut rt = runtime();
    rt.resize(64, 48);
    let full = publish(&mut rt, 0, dir.path(), 0);
    let shm = full.shm.clone().unwrap();
    assert!(full.zlib_rgba.is_empty(), "no pixels on the socket");
    assert!(shm.regions.is_empty(), "first publish is the whole frame");
    assert!(shm.path.starts_with(dir.path()));

    blit(&mut rt, 1, 2, [9, 8, 7, 255]);
    blit(&mut rt, 60, 40, [1, 2, 3, 255]);
    let next = publish(&mut rt, full.revision, dir.path(), 0);
    let shm = next.shm.unwrap();
    let rect = |x, y| VncRegion {
        x,
        y,
        width: 1,
        height: 1,
    };
    assert_eq!(
        shm.regions,
        vec![rect(1, 2), rect(60, 40)],
        "no bounding union"
    );
    let reader = SharedFrameReader::open(&shm.path).unwrap();
    let copies = reader
        .read(
            shm.sequence,
            &[FrameRect {
                x: 60,
                y: 40,
                width: 1,
                height: 1,
            }],
        )
        .unwrap();
    assert_eq!(copies[0].1, vec![1, 2, 3, 255]);

    blit(&mut rt, 3, 3, [5, 5, 5, 255]);
    let behind = publish(&mut rt, full.revision, dir.path(), 0);
    assert!(
        behind.shm.unwrap().regions.is_empty(),
        "a viewer behind the published revision gets the whole frame"
    );
}

#[test]
fn a_resize_moves_to_a_new_file_and_removes_the_old_one() {
    let dir = tempfile::tempdir().unwrap();
    let mut rt = runtime();
    rt.resize(8, 8);
    let first = publish(&mut rt, 0, dir.path(), 0).shm.unwrap().path;
    rt.resize(16, 8);
    let second = publish(&mut rt, 0, dir.path(), 0).shm.unwrap().path;
    assert_ne!(first, second, "a mapped file is never truncated");
    assert!(!first.exists() && second.exists());
    drop(rt);
    assert!(!second.exists(), "the session cleans up its file");
}

#[test]
fn the_cursor_image_is_sent_only_when_it_changed() {
    let dir = tempfile::tempdir().unwrap();
    let mut rt = runtime();
    rt.resize(4, 4);
    rt.cursor = Some(terminator_core::vnc::VncCursor {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
        rgba: "AAAA/w==".into(),
    });
    rt.cursor_serial = 3;
    let first = publish(&mut rt, 0, dir.path(), 0);
    assert!(first.cursor.is_some());
    assert_eq!(first.cursor_serial, 3);
    let known = publish(&mut rt, 0, dir.path(), 3);
    assert!(known.cursor.is_none(), "the viewer already has cursor 3");
}

#[test]
fn a_frame_request_waits_for_a_new_revision_then_answers() {
    let rt = Arc::new(Mutex::new(runtime()));
    let since = relock(&rt).revision;
    let writer = Arc::clone(&rt);
    let bump = thread::spawn(move || {
        thread::sleep(Duration::from_millis(30));
        let mut rt = relock(&writer);
        rt.revision = rt.revision.saturating_add(1);
    });
    let started = Instant::now();
    let guard = shm::wait_for_revision(&rt, since, 400);
    assert!(guard.revision > since);
    assert!(
        started.elapsed() < Duration::from_millis(300),
        "no full wait"
    );
    drop(guard);
    bump.join().unwrap();

    let started = Instant::now();
    let guard = shm::wait_for_revision(&rt, u64::MAX, 20);
    assert!(
        started.elapsed() >= Duration::from_millis(20),
        "bounded wait"
    );
    assert!(
        guard.polled_recently(),
        "a waiting viewer counts as watching"
    );
}
