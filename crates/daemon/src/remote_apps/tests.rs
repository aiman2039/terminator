use std::sync::Arc;
use std::thread;
use std::time::Duration;

use terminator_core::remote_apps::{
    ErrorKind, HostMessage, RemoteAppsStatus, WindowId, WindowRole,
    protocol::{FrameEncoding, Framed, PROTOCOL_VERSION},
};
use terminator_sys::{FrameRect, SharedFrameReader};

use super::{Handle, Runtime, worker::classify, worker::watch_detach};
use terminator_core::remote_apps::{permanent, retry_delay_seconds};

fn runtime(dir: &std::path::Path) -> Runtime {
    Runtime::new("s1".into(), "p1".into(), dir.to_path_buf())
}

fn hello(protocol: u32) -> Framed<HostMessage> {
    Framed::new(HostMessage::Hello {
        protocol,
        version: "0.200.0".into(),
        display: Some(":100".into()),
        wayland_display: "wprs-0".into(),
        capabilities: Vec::new(),
    })
}

fn opened(id: u64) -> Framed<HostMessage> {
    Framed::new(HostMessage::WindowOpened {
        id: WindowId(id),
        role: WindowRole::Toplevel,
        title: "xclock".into(),
        app_id: String::new(),
    })
}

fn frame(id: u64, width: u32, height: u32, bgra: [u8; 4]) -> Framed<HostMessage> {
    let pixels = bgra.repeat(usize::try_from(width.checked_mul(height).unwrap()).unwrap());
    Framed::with_payload(
        HostMessage::Frame {
            id: WindowId(id),
            width,
            height,
            damage: Vec::new(),
            encoding: FrameEncoding::ZstdBgra,
        },
        zstd::bulk::compress(&pixels, 1).unwrap(),
    )
}

#[test]
fn hello_connects_and_records_the_display() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = runtime(dir.path());
    assert!(r.apply(hello(PROTOCOL_VERSION)));
    let state = r.snapshot();
    assert_eq!(state.status, RemoteAppsStatus::Connected);
    assert_eq!(state.display.as_deref(), Some(":100"));
}

#[test]
fn newer_host_protocol_fails_and_names_this_computer_to_update() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = runtime(dir.path());
    r.apply(hello(PROTOCOL_VERSION + 1));
    let state = r.snapshot();
    assert_eq!(state.status, RemoteAppsStatus::Failed);
    assert_eq!(state.error_kind, Some(ErrorKind::VersionMismatch));
    assert!(
        state
            .error
            .unwrap()
            .contains("Update Terminator on this computer")
    );
}

#[test]
fn frame_lands_in_shared_memory_as_rgba_and_close_removes_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = runtime(dir.path());
    r.apply(hello(PROTOCOL_VERSION));
    r.apply(opened(1));
    assert!(r.apply(frame(1, 2, 2, [10, 20, 30, 255])));
    let window = r.snapshot().windows.remove(0);
    assert_eq!((window.width, window.height), (2, 2));
    let path = window.shm.unwrap();
    let reader = SharedFrameReader::open(&path).unwrap();
    let full = FrameRect {
        x: 0,
        y: 0,
        width: 2,
        height: 2,
    };
    let pixels = reader.read(window.sequence, &[full]).unwrap();
    assert_eq!(pixels[0].1, [30, 20, 10, 255].repeat(4));

    r.apply(Framed::new(HostMessage::WindowClosed { id: WindowId(1) }));
    assert!(r.snapshot().windows.is_empty());
    assert!(!path.exists());
}

#[test]
fn resize_moves_the_window_to_a_new_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = runtime(dir.path());
    r.apply(opened(1));
    r.apply(frame(1, 2, 2, [0; 4]));
    let first = r.snapshot().windows[0].shm.clone().unwrap();
    r.apply(frame(1, 4, 3, [0; 4]));
    let window = &r.snapshot().windows[0];
    assert_eq!((window.width, window.height), (4, 3));
    assert_ne!(window.shm.as_ref(), Some(&first));
    assert!(!first.exists());
}

#[test]
fn corrupt_or_unknown_frames_change_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = runtime(dir.path());
    r.apply(opened(1));
    let before = r.snapshot().revision;
    let mut bad = frame(1, 2, 2, [0; 4]);
    bad.payload = b"junk".to_vec();
    assert!(!r.apply(bad));
    assert!(!r.apply(frame(9, 2, 2, [0; 4])));
    assert!(!r.apply(frame(1, 70_000, 1, [0; 4])));
    assert_eq!(r.snapshot().revision, before);
}

#[test]
fn bridge_error_is_kept_when_the_bridge_then_exits() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = runtime(dir.path());
    r.apply(opened(1));
    r.apply(Framed::new(HostMessage::Error {
        kind: ErrorKind::XwaylandMissing,
        message: "Xwayland not found".into(),
    }));
    r.end(
        RemoteAppsStatus::Failed,
        Some((ErrorKind::Other, "exit 2".into())),
    );
    let state = r.snapshot();
    assert_eq!(state.error_kind, Some(ErrorKind::XwaylandMissing));
    assert!(state.windows.is_empty());
}

#[test]
fn popups_are_listed_after_their_parents() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = runtime(dir.path());
    r.apply(Framed::new(HostMessage::WindowOpened {
        id: WindowId(1),
        role: WindowRole::Popup {
            parent: WindowId(2),
            x: 0,
            y: 0,
        },
        title: String::new(),
        app_id: String::new(),
    }));
    r.apply(opened(2));
    let ids: Vec<u64> = r.snapshot().windows.iter().map(|w| w.id.0).collect();
    assert_eq!(ids, [2, 1]);
}

#[test]
fn long_poll_wakes_on_change_and_times_out_without_one() {
    let dir = tempfile::tempdir().unwrap();
    let handle = Arc::new(Handle::new(runtime(dir.path())));
    let since = handle.snapshot().revision;
    assert!(
        handle
            .wait_newer(since, Duration::from_millis(20))
            .is_none()
    );
    let waker = Arc::clone(&handle);
    let poke = thread::spawn(move || {
        thread::sleep(Duration::from_millis(30));
        waker.update(|r| {
            r.apply(opened(1));
        });
    });
    let state = handle.wait_newer(since, Duration::from_secs(5)).unwrap();
    assert_eq!(state.windows.len(), 1);
    poke.join().unwrap();
}

#[test]
fn cursor_image_becomes_rgba_and_each_change_gets_a_new_serial() {
    use base64::Engine as _;
    let dir = tempfile::tempdir().unwrap();
    let mut r = runtime(dir.path());
    let cursor = |payload: Vec<u8>, name: Option<&str>| {
        Framed::with_payload(
            HostMessage::Cursor {
                width: u32::from(name.is_none()),
                height: u32::from(name.is_none()),
                hotspot_x: 3,
                hotspot_y: 4,
                name: name.map(str::to_owned),
            },
            payload,
        )
    };
    assert!(r.apply(cursor(vec![1, 2, 3, 255], None)));
    let first = r.snapshot().cursor.unwrap();
    let rgba = base64::engine::general_purpose::STANDARD
        .decode(&first.rgba)
        .unwrap();
    assert_eq!(rgba, [3, 2, 1, 255]);
    assert_eq!((first.hotspot_x, first.hotspot_y), (3, 4));
    r.apply(cursor(Vec::new(), Some("text")));
    let second = r.snapshot().cursor.unwrap();
    assert_eq!(second.name.as_deref(), Some("text"));
    assert!(second.serial > first.serial);
}

#[test]
fn exit_reasons_name_the_missing_bridge_and_a_stopped_wprsd() {
    let (kind, text) = classify(Some(127), b"sh: terminator-remote-bridge: not found\n");
    assert_eq!(kind, ErrorKind::WprsdMissing);
    assert!(text.contains("not installed on the host"));
    let (kind, text) = classify(Some(1), b"server disconnected: Err(..)\n");
    assert_eq!(kind, ErrorKind::WprsdStopped);
    assert!(text.contains("wprsd stopped"));
    let (kind, _) = classify(None, b"");
    assert_eq!(kind, ErrorKind::Other);
}

#[test]
fn retries_back_off_to_a_minute_and_skip_install_problems() {
    let delays: Vec<u64> = (1..=9).map(retry_delay_seconds).collect();
    assert_eq!(delays, [1, 2, 4, 8, 16, 32, 60, 60, 60]);
    assert!(permanent(ErrorKind::WprsdMissing));
    assert!(permanent(ErrorKind::VersionMismatch));
    assert!(!permanent(ErrorKind::WprsdStopped));
    assert!(!permanent(ErrorKind::Other));
}

#[test]
fn retry_keeps_the_error_closes_windows_and_hello_clears_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = runtime(dir.path());
    r.apply(hello(PROTOCOL_VERSION));
    r.apply(opened(1));
    r.apply(frame(1, 1, 1, [0; 4]));
    r.retrying((ErrorKind::Other, "Connection reset".into()), 2, 2);
    let state = r.snapshot();
    assert_eq!(state.status, RemoteAppsStatus::Connecting);
    assert_eq!(state.error.as_deref(), Some("Connection reset"));
    assert_eq!((state.retry_attempt, state.retry_in_seconds), (2, 2));
    assert!(state.windows.is_empty());
    assert!(!r.hello_seen);

    r.apply(hello(PROTOCOL_VERSION));
    let state = r.snapshot();
    assert_eq!(state.status, RemoteAppsStatus::Connected);
    assert_eq!((state.error, state.retry_attempt), (None, 0));
}

#[test]
fn bridge_error_is_read_back_only_while_failed() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = runtime(dir.path());
    assert!(r.bridge_error().is_none());
    r.apply(Framed::new(HostMessage::Error {
        kind: ErrorKind::WprsdMissing,
        message: "wprsd not found".into(),
    }));
    assert_eq!(r.bridge_error().map(|e| e.0), Some(ErrorKind::WprsdMissing));
}

#[test]
fn stop_request_wakes_a_waiting_retry() {
    let dir = tempfile::tempdir().unwrap();
    let handle = Arc::new(Handle::new(runtime(dir.path())));
    assert!(!handle.wait_stop(Duration::from_millis(10)));
    let stopper = Arc::clone(&handle);
    let stop = thread::spawn(move || {
        thread::sleep(Duration::from_millis(30));
        stopper.request_stop();
    });
    assert!(handle.wait_stop(Duration::from_secs(5)));
    assert!(handle.read(|r| r.input.is_none()));
    stop.join().unwrap();
}

#[test]
fn detach_timeout_stops_a_session_nobody_views() {
    let dir = tempfile::tempdir().unwrap();
    let handle = Handle::new(runtime(dir.path()));
    let limit: super::worker::DetachLimit = Box::new(|| Some(Duration::from_millis(20)));
    watch_detach(&handle, &limit, Duration::from_millis(30));
    assert!(handle.read(|r| r.stop_requested));
}

#[test]
fn detach_timeout_of_never_keeps_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let handle = Arc::new(Handle::new(runtime(dir.path())));
    let watched = Arc::clone(&handle);
    let watcher = thread::spawn(move || {
        let never: super::worker::DetachLimit = Box::new(|| None);
        watch_detach(&watched, &never, Duration::from_millis(10));
    });
    thread::sleep(Duration::from_millis(60));
    assert!(!handle.read(|r| r.stop_requested));
    // The watcher ends with the session.
    handle.update(|r| r.end(RemoteAppsStatus::Disconnected, None));
    watcher.join().unwrap();
}
