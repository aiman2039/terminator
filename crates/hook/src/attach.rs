//! `terminator-hook attach`: bridge stdio to a daemon-owned terminal session.
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
#[cfg(windows)]
use std::time::Duration;
use std::{
    io::{Read, Write},
    sync::{Arc, Mutex},
};
use terminator_core::*;

type SharedWriter = Arc<Mutex<transport::Stream>>;

#[cfg(unix)]
struct Raw(rustix::termios::Termios);
#[cfg(unix)]
impl Drop for Raw {
    fn drop(&mut self) {
        let _ = rustix::termios::tcsetattr(
            std::io::stdin(),
            rustix::termios::OptionalActions::Now,
            &self.0,
        );
    }
}

pub(crate) struct Attach;
impl Attach {
    pub(crate) fn run(session: &str, args: &[String]) -> Result<()> {
        let raw = Self::raw_mode();
        let paths = Self::paths(args)?;
        let (rows, cols) = Self::dimensions();
        let mut stream = connect(
            &paths,
            Request::Attach {
                session: session.into(),
                rows,
                cols,
            },
            None,
        )?;
        stream.set_read_timeout(None)?;
        let writer = Arc::new(Mutex::new(stream.try_clone()?));
        Self::spawn_input(Arc::clone(&writer));
        Self::spawn_resize(&writer, (rows, cols))?;
        // Attach used the PTY size at spawn (often 80x50). Catch a GUI resize that
        // raced before this handler existed so COLUMNS matches the painted grid.
        let _ = Self::write_winsize(writer.as_ref());
        Self::pump_output(&mut stream)?;
        drop(raw);
        Ok(())
    }
    #[cfg(unix)]
    fn raw_mode() -> Option<Raw> {
        rustix::termios::tcgetattr(std::io::stdin())
            .ok()
            .map(|previous| {
                let mut mode = previous.clone();
                mode.make_raw();
                let _ = rustix::termios::tcsetattr(
                    std::io::stdin(),
                    rustix::termios::OptionalActions::Now,
                    &mode,
                );
                Raw(previous)
            })
    }
    // Without a console (redirected stdio) there is no mode to set; the
    // bridge still works, but interactive programs may echo input. The
    // guard restores the previous console mode when dropped.
    #[cfg(not(unix))]
    fn raw_mode() -> Option<terminator_sys::RawMode> {
        terminator_sys::RawMode::enable().ok()
    }
    fn paths(args: &[String]) -> Result<Paths> {
        if args.len() >= 2 {
            return Ok(Paths {
                data: args.first().context("Missing data path")?.clone().into(),
                runtime: args.get(1).context("Missing runtime path")?.clone().into(),
            });
        }
        Paths::discover()
    }
    fn spawn_input(input_writer: SharedWriter) {
        std::thread::spawn(move || {
            let mut input = std::io::stdin();
            let mut bytes = [0; 8192];
            while let Ok(n) = input.read(&mut bytes) {
                if n == 0 {
                    break;
                }
                let Some(chunk) = bytes.get(..n) else {
                    break;
                };
                if write_frame(
                    &mut *Self::lock_writer(&input_writer),
                    &Request::Input {
                        data: B64.encode(chunk),
                    },
                )
                .is_err()
                {
                    break;
                }
            }
            let _ = Self::lock_writer(&input_writer).shutdown();
        });
    }
    #[cfg(unix)]
    fn spawn_resize(writer: &SharedWriter, _size: (u16, u16)) -> Result<()> {
        let mut signals = signal_hook::iterator::Signals::new([signal_hook::consts::SIGWINCH])?;
        let sigwinch_writer = Arc::clone(writer);
        std::thread::spawn(move || {
            for _ in signals.forever() {
                if Self::write_winsize(sigwinch_writer.as_ref()).is_err() {
                    break;
                }
            }
        });
        Ok(())
    }
    // Windows has no SIGWINCH; poll the console size instead and forward
    // `Request::Resize` on change, like the signal handler does on Unix.
    #[cfg(windows)]
    fn spawn_resize(writer: &SharedWriter, size: (u16, u16)) -> Result<()> {
        let resize_writer = Arc::clone(writer);
        let mut known = size;
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(200));
                if Self::resize_changed(&mut known, Self::dimensions())
                    && Self::write_winsize(resize_writer.as_ref()).is_err()
                {
                    break;
                }
            }
        });
        Ok(())
    }
    fn pump_output(stream: &mut transport::Stream) -> Result<()> {
        let mut output = std::io::stdout();
        while let Ok(frame) = read_frame::<Response>(stream) {
            match frame {
                Response::Data(data) => {
                    output.write_all(&B64.decode(data)?)?;
                    output.flush()?;
                }
                Response::End => break,
                Response::Error(e) => bail!("{e}"),
                _ => {}
            }
        }
        Ok(())
    }
    #[cfg(unix)]
    fn dimensions() -> (u16, u16) {
        let size =
            rustix::termios::tcgetwinsize(std::io::stdin()).unwrap_or(rustix::termios::Winsize {
                ws_row: 24,
                ws_col: 80,
                ws_xpixel: 0,
                ws_ypixel: 0,
            });
        (size.ws_row.max(1), size.ws_col.max(1))
    }
    #[cfg(not(unix))]
    fn dimensions() -> (u16, u16) {
        terminator_sys::dimensions()
    }
    /// Track the last forwarded size; true when `current` is a new resize.
    /// Resize frames go out on change only, so an idle poll loop never floods
    /// the daemon with identical sizes. Live on Windows, covered by tests
    /// everywhere else.
    #[cfg(any(windows, test))]
    fn resize_changed(known: &mut (u16, u16), current: (u16, u16)) -> bool {
        if *known == current {
            return false;
        }
        *known = current;
        true
    }
    fn lock_writer<T>(writer: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    fn write_winsize(writer: &Mutex<impl Write>) -> Result<()> {
        let (rows, cols) = Self::dimensions();
        write_frame(
            &mut *Self::lock_writer(writer),
            &Request::Resize { rows, cols },
        )
    }
}

#[cfg(test)]
mod winsize_tests {
    use super::*;
    #[test]
    fn resizes_forward_once_per_size_change() {
        let mut known = (24, 80);
        assert!(!Attach::resize_changed(&mut known, (24, 80)));
        assert_eq!(known, (24, 80));
        assert!(Attach::resize_changed(&mut known, (30, 100)));
        assert_eq!(known, (30, 100));
        assert!(!Attach::resize_changed(&mut known, (30, 100)));
    }
    #[test]
    fn write_winsize_sends_a_resize_frame() {
        let buf = Mutex::new(Vec::new());
        assert!(Attach::write_winsize(&buf).is_ok());
        let bytes = buf.into_inner().unwrap_or_default();
        assert!(!bytes.is_empty());
        let req = read_frame::<Request>(&mut bytes.as_slice());
        assert!(
            matches!(req, Ok(Request::Resize { rows, cols }) if rows >= 1 && cols >= 1),
            "{req:?}"
        );
    }
}
