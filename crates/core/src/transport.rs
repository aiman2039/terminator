//! Local IPC transport: Unix sockets on Unix, TCP loopback on Windows.
//!
//! The standard library has no Unix sockets on Windows, so the daemon binds
//! 127.0.0.1 on an ephemeral port and records `ip:port` in the file at the
//! socket path. Callers keep passing [`crate::Paths::socket`] (and the
//! `gui.sock` path); on Windows that file holds the address instead of being
//! a socket. Every frame still carries the auth token, and the port file lives
//! in the mode-0700 equivalent runtime directory, so a loopback listener does
//! not widen access beyond local processes that already share the data dir.

use anyhow::{Context, Result};
use std::{
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    path::Path,
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

/// Default IPC timeouts, matching the previous sync client behavior.
pub const READ_TIMEOUT: Duration = Duration::from_secs(3);
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(3);

/// A connected IPC stream. `Unix` on Unix, TCP loopback on Windows.
pub enum Stream {
    #[cfg(unix)]
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl Stream {
    /// Connect to the daemon (or GUI) endpoint described by `socket_path`.
    pub fn connect_ipc(socket_path: &Path) -> Result<Self> {
        #[cfg(unix)]
        {
            let stream = UnixStream::connect(socket_path).context("Session daemon unavailable")?;
            Ok(Self::Unix(stream))
        }
        #[cfg(not(unix))]
        {
            let addr = read_port(socket_path)?;
            let stream = TcpStream::connect(addr).context("Session daemon unavailable")?;
            Ok(Self::Tcp(stream))
        }
    }

    /// Apply read/write timeouts. The daemon relies on these so a wedged peer
    /// cannot hold a handler thread forever.
    pub fn set_timeouts(&self, read: Duration, write: Duration) -> Result<()> {
        self.set_read_timeout(Some(read))?;
        self.set_write_timeout(Some(write))
    }

    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(s) => s.set_read_timeout(timeout)?,
            Self::Tcp(s) => s.set_read_timeout(timeout)?,
        }
        Ok(())
    }

    pub fn set_write_timeout(&self, timeout: Option<Duration>) -> Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(s) => s.set_write_timeout(timeout)?,
            Self::Tcp(s) => s.set_write_timeout(timeout)?,
        }
        Ok(())
    }

    pub fn set_nonblocking(&self, nonblocking: bool) -> Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(s) => s.set_nonblocking(nonblocking)?,
            Self::Tcp(s) => s.set_nonblocking(nonblocking)?,
        }
        Ok(())
    }

    pub fn try_clone(&self) -> Result<Self> {
        match self {
            #[cfg(unix)]
            Self::Unix(s) => Ok(Self::Unix(s.try_clone()?)),
            Self::Tcp(s) => Ok(Self::Tcp(s.try_clone()?)),
        }
    }

    pub fn shutdown(&self) -> Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(s) => s.shutdown(Shutdown::Both)?,
            Self::Tcp(s) => s.shutdown(Shutdown::Both)?,
        }
        Ok(())
    }

    /// Hand the socket to a child process' stdio (restart helper tracking).
    #[cfg(unix)]
    pub fn into_owned_fd(self) -> std::os::fd::OwnedFd {
        match self {
            Self::Unix(s) => s.into(),
            Self::Tcp(s) => s.into(),
        }
    }

    /// Hand the socket to a child process' stdio (restart helper tracking).
    #[cfg(not(unix))]
    pub fn into_owned_fd(self) -> std::os::windows::io::OwnedHandle {
        match self {
            Self::Tcp(s) => terminator_sys::socket_into_handle(s),
        }
    }

    /// Hand the socket to Tokio (restart completion observer). Tokio
    /// requires the socket to be non-blocking; this sets that first.
    #[cfg(feature = "async-client")]
    pub fn into_tokio(self) -> std::io::Result<AsyncStream> {
        self.set_nonblocking(true).map_err(std::io::Error::other)?;
        match self {
            #[cfg(unix)]
            Self::Unix(s) => Ok(AsyncStream::Unix(tokio::net::UnixStream::from_std(s)?)),
            Self::Tcp(s) => Ok(AsyncStream::Tcp(tokio::net::TcpStream::from_std(s)?)),
        }
    }

    /// Borrow the inner Unix socket for `poll`-based loops (Unix only).
    #[cfg(unix)]
    pub fn as_std(&self) -> &UnixStream {
        match self {
            Self::Unix(s) => s,
            Self::Tcp(_) => unreachable!("TCP variant never binds on Unix"),
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(unix)]
            Self::Unix(s) => s.read(buf),
            Self::Tcp(s) => s.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(unix)]
            Self::Unix(s) => s.write(buf),
            Self::Tcp(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(s) => s.flush(),
            Self::Tcp(s) => s.flush(),
        }
    }
}

/// A bound IPC endpoint. `Unix` on Unix, TCP loopback on Windows.
pub enum Listener {
    #[cfg(unix)]
    Unix(UnixListener),
    Tcp(TcpListener),
}

impl Listener {
    /// Bind the endpoint described by `socket_path`, replacing any stale one.
    pub fn bind_ipc(socket_path: &Path) -> Result<Self> {
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(socket_path);
            let listener = UnixListener::bind(socket_path)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600))?;
            }
            Ok(Self::Unix(listener))
        }
        #[cfg(not(unix))]
        {
            let _ = std::fs::remove_file(socket_path);
            let listener = TcpListener::bind("127.0.0.1:0")?;
            let addr = listener.local_addr()?;
            crate::atomic_write(socket_path, addr.to_string().as_bytes())?;
            Ok(Self::Tcp(listener))
        }
    }

    pub fn set_nonblocking(&self, nonblocking: bool) -> Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(l) => l.set_nonblocking(nonblocking)?,
            Self::Tcp(l) => l.set_nonblocking(nonblocking)?,
        }
        Ok(())
    }

    /// Borrow the inner Unix listener for `poll`-based loops (Unix only).
    #[cfg(unix)]
    pub fn as_std(&self) -> &UnixListener {
        match self {
            Self::Unix(l) => l,
            Self::Tcp(_) => unreachable!("TCP variant never binds on Unix"),
        }
    }

    /// Accept one connection. The string is a peer description for logging.
    /// `WouldBlock` surfaces unchanged so non-blocking loops keep working.
    pub fn accept(&self) -> std::io::Result<(Stream, String)> {
        match self {
            #[cfg(unix)]
            Self::Unix(l) => {
                let (stream, addr) = l.accept()?;
                Ok((Stream::Unix(stream), format!("{addr:?}")))
            }
            Self::Tcp(l) => {
                let (stream, addr) = l.accept()?;
                Ok((Stream::Tcp(stream), addr.to_string()))
            }
        }
    }
}

/// Remove the endpoint record: socket file on Unix, port file on Windows.
pub fn cleanup_ipc(socket_path: &Path) {
    let _ = std::fs::remove_file(socket_path);
}

/// Neovim `--listen` address for an editor socket path: the path itself on
/// Unix, a deterministic named pipe on Windows (Neovim and the RPC client
/// derive the same name from the same path).
pub fn nvim_listen_arg(socket_path: &Path) -> std::ffi::OsString {
    #[cfg(unix)]
    {
        socket_path.as_os_str().to_os_string()
    }
    #[cfg(not(unix))]
    {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        socket_path.as_os_str().hash(&mut hasher);
        std::ffi::OsString::from(format!(
            r"\\.\pipe\terminator-nvim-{:016x}",
            hasher.finish()
        ))
    }
}

/// A live endpoint answers a bare connection. Used for generation liveness.
pub fn probe_ipc(socket_path: &Path) -> bool {
    Stream::connect_ipc(socket_path).is_ok()
}

/// A connected pair for in-process wake signaling (replaces `UnixStream::pair`).
pub fn pair() -> Result<(Stream, Stream)> {
    #[cfg(unix)]
    {
        let (a, b) = UnixStream::pair()?;
        Ok((Stream::Unix(a), Stream::Unix(b)))
    }
    #[cfg(not(unix))]
    {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let a = TcpStream::connect(addr)?;
        let (b, _) = listener.accept()?;
        Ok((Stream::Tcp(a), Stream::Tcp(b)))
    }
}

#[cfg(not(unix))]
fn read_port(socket_path: &Path) -> Result<std::net::SocketAddr> {
    std::fs::read_to_string(socket_path)
        .context("Session daemon unavailable")?
        .trim()
        .parse()
        .context("Session daemon address is corrupt")
}

/// Async connected IPC stream for the Tokio client.
#[cfg(feature = "async-client")]
pub enum AsyncStream {
    #[cfg(unix)]
    Unix(tokio::net::UnixStream),
    Tcp(tokio::net::TcpStream),
}

#[cfg(feature = "async-client")]
impl AsyncStream {
    pub async fn connect_ipc(socket_path: &Path) -> Result<Self> {
        #[cfg(unix)]
        {
            let stream = tokio::net::UnixStream::connect(socket_path)
                .await
                .context("Session daemon unavailable")?;
            Ok(Self::Unix(stream))
        }
        #[cfg(not(unix))]
        {
            // A tiny local file read; Tokio is built without `fs` on purpose.
            let addr = read_port(socket_path)?;
            let stream = tokio::net::TcpStream::connect(addr)
                .await
                .context("Session daemon unavailable")?;
            Ok(Self::Tcp(stream))
        }
    }
}

#[cfg(feature = "async-client")]
impl tokio::io::AsyncRead for AsyncStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            #[cfg(unix)]
            Self::Unix(s) => std::pin::Pin::new(s).poll_read(cx, buf),
            Self::Tcp(s) => std::pin::Pin::new(s).poll_read(cx, buf),
        }
    }
}

#[cfg(feature = "async-client")]
impl tokio::io::AsyncWrite for AsyncStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match self.get_mut() {
            #[cfg(unix)]
            Self::Unix(s) => std::pin::Pin::new(s).poll_write(cx, buf),
            Self::Tcp(s) => std::pin::Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            #[cfg(unix)]
            Self::Unix(s) => std::pin::Pin::new(s).poll_flush(cx),
            Self::Tcp(s) => std::pin::Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            #[cfg(unix)]
            Self::Unix(s) => std::pin::Pin::new(s).poll_shutdown(cx),
            Self::Tcp(s) => std::pin::Pin::new(s).poll_shutdown(cx),
        }
    }
}

/// Async bound IPC endpoint for tests and the GUI control socket.
#[cfg(feature = "async-client")]
pub enum AsyncListener {
    #[cfg(unix)]
    Unix(tokio::net::UnixListener),
    Tcp(tokio::net::TcpListener),
}

#[cfg(feature = "async-client")]
impl AsyncListener {
    /// Adopt a synchronously bound [`Listener`]. Binding must happen on a
    /// native worker thread (Tokio constructors panic without a runtime), and
    /// this conversion must run inside the Tokio runtime that will poll it.
    pub fn from_std(listener: Listener) -> Result<Self> {
        match listener {
            #[cfg(unix)]
            Listener::Unix(listener) => {
                listener.set_nonblocking(true)?;
                Ok(Self::Unix(tokio::net::UnixListener::from_std(listener)?))
            }
            Listener::Tcp(listener) => {
                listener.set_nonblocking(true)?;
                Ok(Self::Tcp(tokio::net::TcpListener::from_std(listener)?))
            }
        }
    }

    pub fn bind_ipc(socket_path: &Path) -> Result<Self> {
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(socket_path);
            let listener = tokio::net::UnixListener::bind(socket_path)?;
            Ok(Self::Unix(listener))
        }
        #[cfg(not(unix))]
        {
            let _ = std::fs::remove_file(socket_path);
            let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
            listener.set_nonblocking(true)?;
            let addr = listener.local_addr()?;
            crate::atomic_write(socket_path, addr.to_string().as_bytes())?;
            let listener = tokio::net::TcpListener::from_std(listener)?;
            Ok(Self::Tcp(listener))
        }
    }

    pub async fn accept(&self) -> Result<(AsyncStream, String)> {
        match self {
            #[cfg(unix)]
            Self::Unix(l) => {
                let (stream, addr) = l.accept().await?;
                Ok((AsyncStream::Unix(stream), format!("{addr:?}")))
            }
            Self::Tcp(l) => {
                let (stream, addr) = l.accept().await?;
                Ok((AsyncStream::Tcp(stream), addr.to_string()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
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
}
