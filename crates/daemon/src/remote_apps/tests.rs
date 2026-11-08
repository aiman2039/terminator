use std::sync::Arc;
use std::thread;
use std::time::Duration;

use terminator_core::remote_apps::{
    ErrorKind, HostMessage, RemoteAppsStatus, WindowId, WindowRole,
    protocol::{FrameEncoding, Framed, PROTOCOL_VERSION},
};
use terminator_sys::{FrameRect, SharedFrameReader};

use super::{Handle, Runtime, worker::classify};

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
