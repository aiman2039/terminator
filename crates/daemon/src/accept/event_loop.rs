use anyhow::Result;
#[cfg(unix)]
use rustix::event::{PollFd, PollFlags, Timespec, poll};
#[cfg(unix)]
use rustix::io::Errno;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};
use terminator_core::transport;

use crate::notifications;
use crate::server::{Shared, drain_wake, serve};

/// The main thread's connection loop, run until shutdown is requested.
pub(crate) struct AcceptLoop<'a> {
    pub listener: &'a transport::Listener,
    pub shared: &'a Arc<Shared>,
    /// Connection handlers currently running.
    pub active: &'a Arc<AtomicUsize>,
    pub wake_drain: &'a mut transport::Stream,
    #[cfg_attr(
        not(unix),
        expect(dead_code, reason = "only the unix poll set waits on the waker")
    )]
    pub wake_rx: &'a transport::Stream,
}

impl AcceptLoop<'_> {
    // Event-driven accept loop: the main thread blocks in `poll` with no
    // timeout while no notification waiter exists, so an idle daemon does no
    // periodic work. A socketpair wakes it when a waiter needs Cocoa run-loop
    // pumping or has finished; a shutdown request wakes it explicitly. The
    // listener is still non-blocking so the poll/accept pair never blocks.
    #[cfg(unix)]
    pub(crate) fn run(mut self) -> Result<()> {
        while !self.shared.shutdown.load(Ordering::Relaxed) {
            self.poll_once()?;
        }
        Ok(())
    }

    // Windows has no `poll` on sockets here; the loop polls the non-blocking
    // listener and waker on a short sleep instead.
    #[cfg(not(unix))]
    pub(crate) fn run(self) -> Result<()> {
        while !self.shared.shutdown.load(Ordering::Relaxed) {
            drain_wake(self.wake_drain);
            if notifications::waiting() {
                notifications::pump();
            }
            self.accept_one()?;
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        Ok(())
    }

    #[cfg(unix)]
    fn poll_once(&mut self) -> Result<()> {
        let waiting = notifications::waiting();
        // While a waiter exists, service the Cocoa run loop often enough that
        // click callbacks and the 0.5 s dismiss poll are not starved, without
        // the previous ~100 wakeups/s. The pump itself is a 1 ms slice.
        let timeout = waiting.then_some(Timespec {
            tv_sec: 0,
            tv_nsec: 100_000_000,
        });
        let mut fds = [
            PollFd::new(self.listener.as_std(), PollFlags::IN),
            PollFd::new(self.wake_rx.as_std(), PollFlags::IN),
        ];
        match poll(&mut fds, timeout.as_ref()) {
            Ok(0) => notifications::pump(),
            Ok(_) => {
                let incoming = fds[0].revents().contains(PollFlags::IN);
                let woken = fds[1].revents().contains(PollFlags::IN);
                self.on_ready(incoming, woken, waiting)?;
            }
            Err(Errno::INTR) => notifications::pump(),
            Err(e @ (Errno::AGAIN | Errno::NOMEM)) => super::back_off(&e.into()),
            Err(e) => return Err(e.into()),
        }
        Ok(())
    }

    #[cfg(unix)]
    fn on_ready(&mut self, incoming: bool, woken: bool, waiting: bool) -> Result<()> {
        if woken {
            drain_wake(self.wake_drain);
        }
        if incoming {
            self.accept_one()?;
        }
        if waiting {
            notifications::pump();
        }
        Ok(())
    }

    fn accept_one(&self) -> Result<()> {
        match self.listener.accept() {
            Ok((stream, _)) => self.spawn_connection(stream),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) if !super::is_fatal(&e) => super::back_off(&e),
            Err(e) => return Err(e.into()),
        }
        Ok(())
    }

    fn spawn_connection(&self, stream: transport::Stream) {
        if self.active.load(Ordering::Relaxed) > 256 {
            return;
        }
        let s = Arc::clone(self.shared);
        let count = Arc::clone(self.active);
        count.fetch_add(1, Ordering::Relaxed);
        thread::spawn(move || {
            // A panicking request kills at most its own connection; the
            // daemon keeps serving. (The crash hook only dumps main-thread
            // panics, so this stays quiet apart from the line below.)
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| serve(stream, s))).is_err()
            {
                crate::console::Console::err(format_args!(
                    "daemon connection handler panicked; connection dropped"
                ));
            }
            count.fetch_sub(1, Ordering::Relaxed);
        });
    }
}
