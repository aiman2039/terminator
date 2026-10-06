//! Windows handle plumbing that needs `unsafe`.
//!
//! Sockets are inheritable handle values on Windows, so a connected loopback
//! socket can stand in for the inherited stdio descriptor the restart helper
//! keeps. There is no safe `TcpStream` to `OwnedHandle` conversion, so the one
//! `unsafe` transfer lives here; every other crate stays `forbid(unsafe_code)`.

/// Move a connected TCP socket into an owned Windows handle without closing
/// it, so a spawned child can inherit exactly one end.
pub fn socket_into_handle(socket: std::net::TcpStream) -> std::os::windows::io::OwnedHandle {
    use std::os::windows::io::{FromRawHandle, IntoRawSocket};
    // SAFETY: `into_raw_socket` relinquishes ownership without closing, and
    // `from_raw_handle` takes ownership of the same value. A `SOCKET` is a
    // valid handle value for inheritance, and nothing else touches it after
    // this move.
    unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(socket.into_raw_socket() as _) }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::*;
    #[cfg(windows)]
    use std::os::windows::io::AsRawHandle;

    #[cfg(windows)]
    #[test]
    fn converted_socket_is_a_valid_handle() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = std::net::TcpStream::connect(addr).unwrap();
        let handle = socket_into_handle(client);
        let raw = handle.as_raw_handle();
        // Null and -1 (`INVALID_HANDLE_VALUE`) both mean "no handle".
        assert!(!raw.is_null());
        assert_ne!(raw as isize, -1);
    }
}
