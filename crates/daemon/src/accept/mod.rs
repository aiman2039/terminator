//! Accept-loop resilience: transient `accept` failures (descriptor
//! exhaustion, aborted handshakes) must not end the daemon, because exiting
//! closes every PTY master and hangs up all live sessions.

mod event_loop;

pub(crate) use event_loop::AcceptLoop;

use std::{io, thread, time::Duration};

/// Back-off after a transient accept failure so descriptor exhaustion does
/// not spin the loop while connections drain.
pub(crate) const BACKOFF: Duration = Duration::from_millis(50);

/// Whether an `accept` error means the listener itself is unusable.
pub(crate) fn is_fatal(error: &io::Error) -> bool {
    if matches!(
        error.kind(),
        io::ErrorKind::WouldBlock
            | io::ErrorKind::Interrupted
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::OutOfMemory
    ) {
        return false;
    }
    #[cfg(unix)]
    if let Some(code) = error.raw_os_error() {
        use rustix::io::Errno;
        let errno = Errno::from_raw_os_error(code);
        return !matches!(
            errno,
            Errno::MFILE | Errno::NFILE | Errno::NOBUFS | Errno::NOMEM | Errno::PROTO
        );
    }
    true
}

/// Log a transient accept failure and pause briefly before retrying.
pub(crate) fn back_off(error: &io::Error) {
    crate::console::Console::err(format_args!(
        "daemon accept failed transiently: {error}; retrying"
    ));
    thread::sleep(BACKOFF);
}

/// Raise the soft descriptor limit toward the hard limit. A GUI-launched
/// daemon on macOS inherits a 256 soft limit, which a few dozen sessions with
/// attached views exhaust.
#[cfg(unix)]
pub(crate) fn raise_fd_limit() {
    use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
    // macOS rejects soft limits above OPEN_MAX even with an unlimited hard
    // limit.
    const CEILING: u64 = 10_240;
    let current = getrlimit(Resource::Nofile);
    let Some(soft) = current.current else {
        return;
    };
    let target = current.maximum.map_or(CEILING, |hard| hard.min(CEILING));
    if target <= soft {
        return;
    }
    if let Err(error) = setrlimit(
        Resource::Nofile,
        Rlimit {
            current: Some(target),
            maximum: current.maximum,
        },
    ) {
        crate::console::Console::err(format_args!(
            "daemon could not raise descriptor limit to {target}: {error}"
        ));
    }
}

#[cfg(not(unix))]
pub(crate) fn raise_fd_limit() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_accept_errors_keep_the_daemon_running() {
        for kind in [
            io::ErrorKind::WouldBlock,
            io::ErrorKind::Interrupted,
            io::ErrorKind::ConnectionAborted,
            io::ErrorKind::ConnectionReset,
        ] {
            assert!(!is_fatal(&io::Error::from(kind)), "{kind:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn descriptor_exhaustion_is_transient() {
        for code in [libc_like::EMFILE, libc_like::ENFILE, libc_like::ENOBUFS] {
            assert!(!is_fatal(&io::Error::from_raw_os_error(code)), "{code}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_broken_listener_is_fatal() {
        assert!(is_fatal(&io::Error::from_raw_os_error(libc_like::EBADF)));
        assert!(is_fatal(&io::Error::from_raw_os_error(libc_like::EINVAL)));
    }

    #[cfg(unix)]
    #[test]
    fn raising_the_limit_never_lowers_it() {
        use rustix::process::{Resource, getrlimit};
        let before = getrlimit(Resource::Nofile).current;
        raise_fd_limit();
        let after = getrlimit(Resource::Nofile).current;
        assert!(after >= before);
    }

    #[cfg(unix)]
    mod libc_like {
        use rustix::io::Errno;
        pub(super) const EMFILE: i32 = Errno::MFILE.raw_os_error();
        pub(super) const ENFILE: i32 = Errno::NFILE.raw_os_error();
        pub(super) const ENOBUFS: i32 = Errno::NOBUFS.raw_os_error();
        pub(super) const EBADF: i32 = Errno::BADF.raw_os_error();
        pub(super) const EINVAL: i32 = Errno::INVAL.raw_os_error();
    }
}
