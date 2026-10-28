use super::*;

/// The loopback transport is the Windows path; exercise it on every OS so
/// Windows IPC has coverage without a Windows host.
fn loopback_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let client = TcpStream::connect(addr).unwrap();
    let (server, _) = listener.accept().unwrap();
    (client, server)
}

#[test]
fn loopback_pair_relays_frames() {
    let (mut a, mut b) = loopback_pair();
    crate::write_frame(&mut a, &serde_json::json!({"hello": "windows"})).unwrap();
    let back: serde_json::Value = crate::read_frame(&mut b).unwrap();
    assert_eq!(back["hello"], "windows");
}

#[test]
fn bind_accept_and_probe_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("daemon.sock");
    let listener = Listener::bind_ipc(&path).unwrap();
    assert!(probe_ipc(&path));
    // `probe_ipc` leaves a pending connection; drain it first.
    listener.set_nonblocking(true).unwrap();
    while listener.accept().is_ok() {}
    listener.set_nonblocking(false).unwrap();
    let handle = std::thread::spawn({
        let path = path.clone();
        move || {
            let mut client = Stream::connect_ipc(&path).unwrap();
            crate::write_frame(&mut client, &42u32).unwrap();
        }
    });
    let (mut server, _) = listener.accept().unwrap();
    let value: u32 = crate::read_frame(&mut server).unwrap();
    assert_eq!(value, 42);
    handle.join().unwrap();
    cleanup_ipc(&path);
    assert!(!probe_ipc(&path));
}

/// The GUI control socket binds on a native worker thread with no Tokio
/// runtime, then adopts the socket inside Tokio. Binding synchronously
/// outside a runtime must work, and conversion inside one must accept.
#[test]
#[cfg(feature = "async-client")]
fn sync_bind_converts_to_async_inside_tokio() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gui.sock");
    // No Tokio runtime here, mirroring the native catalog worker.
    let std_listener = Listener::bind_ipc(&path).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async move {
        let listener = AsyncListener::from_std(std_listener).unwrap();
        let path_clone = path.clone();
        let client =
            tokio::spawn(async move { AsyncStream::connect_ipc(&path_clone).await.unwrap() });
        let (_server, _) = listener.accept().await.unwrap();
        client.await.unwrap();
    });
}

#[test]
fn nvim_listen_arg_is_stable_and_unique_per_socket() {
    let first = nvim_listen_arg(Path::new("/tmp/rt/aaa.nvim"));
    let again = nvim_listen_arg(Path::new("/tmp/rt/aaa.nvim"));
    let other = nvim_listen_arg(Path::new("/tmp/rt/bbb.nvim"));
    assert_eq!(first, again);
    assert_ne!(first, other);
    #[cfg(unix)]
    assert_eq!(first, Path::new("/tmp/rt/aaa.nvim").as_os_str());
    #[cfg(windows)]
    assert!(
        first
            .to_str()
            .is_some_and(|s| s.starts_with(r"\\.\pipe\terminator-nvim-"))
    );
}

#[test]
fn wake_pair_delivers_a_byte() {
    let (mut tx, mut rx) = pair().unwrap();
    tx.set_nonblocking(false).unwrap();
    rx.set_nonblocking(false).unwrap();
    tx.write_all(&[1u8]).unwrap();
    let mut buf = [0u8; 1];
    rx.read_exact(&mut buf).unwrap();
    assert_eq!(buf, [1u8]);
}
