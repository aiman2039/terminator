#![forbid(unsafe_code)]
mod browser;
mod control;
mod shutdown;
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use std::{
    io::{Read, Write},
    sync::{Arc, Mutex},
    time::Duration,
};
use terminator_core::*;
fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(String::as_str) != Some("--health-check")
        && let Ok(paths) = Paths::discover()
    {
        crash::install(crash::CrashInstall {
            binary: "terminator-hook",
            version: env!("CARGO_PKG_VERSION"),
            data_dir: paths.data,
            notify: None,
        });
    }
    match args.first().map(String::as_str) {
        Some("--health-check") => {
            println!("{}:{}", env!("CARGO_PKG_VERSION"), PROTOCOL_VERSION);
            Ok(())
        }
        Some("ctl") => control::run(args.get(1..).unwrap_or_default()),
        Some("attach") => attach(
            args.get(1).context("Missing session ID")?,
            args.get(2..).unwrap_or_default(),
        ),
        Some("event" | "emit" | "cwd" | "prompt") => {
            // Observational hooks never block or alter an agent's decision.
            let _ = hook(&args);
            Ok(())
        }
        Some("rpc") => {
            let req: Request = serde_json::from_str(args.get(1).context("Expected JSON request")?)?;
            let response = rpc(&generations::workspace_paths(&Paths::discover()?)?, req)?;
            println!("{}", serde_json::to_string(&response)?);
            Ok(())
        }
        Some("install" | "remove" | "inspect") => {
            let home = std::env::var_os("TERMINATOR_CONFIG_HOME")
                .or(std::env::var_os("HOME"))
                .context("No user directory")?;
            let kind = args.get(1).context("Expected agent kind")?;
            if args.first().is_some_and(|arg| arg == "inspect") {
                println!(
                    "{}",
                    terminator_integrations::installed(std::path::Path::new(&home), kind)
                );
            } else {
                let path = terminator_integrations::install(
                    std::path::Path::new(&home),
                    kind,
                    &std::env::current_exe()?,
                    args.first().is_some_and(|arg| arg == "remove"),
                )?;
                println!("{}", path.display());
            }
            Ok(())
        }
        _ => {
            println!(
                "terminator-hook ctl --help\nterminator-hook attach SESSION\nterminator-hook event AGENT < payload.json\nterminator-hook emit < normalized-event.json\nterminator-hook cwd PATH\nterminator-hook install|remove|inspect AGENT\nterminator-hook rpc JSON\n\nHooks are inert outside app-created terminals. They never approve or deny agent actions."
            );
            Ok(())
        }
    }
}
fn hook(args: &[String]) -> Result<()> {
    let command = args.first().context("Missing hook command")?;
    let mut paths = Paths::discover()?;
    if let Some(i) = args.iter().position(|a| a == "--data-dir") {
        let next = i.checked_add(1).context("Missing data path")?;
        paths.data = args.get(next).context("Missing data path")?.into();
    }
    if let Some(i) = args.iter().position(|a| a == "--runtime-dir") {
        let next = i.checked_add(1).context("Missing runtime path")?;
        paths.runtime = args.get(next).context("Missing runtime path")?.into();
    }
    let (parent, actual, ancestors, process) = if command == "event"
        || command == "emit"
        || std::env::var_os("TERMINATOR_SESSION_ID").is_none()
    {
        agent_parent()
    } else {
        (String::new(), None, vec![], None)
    };
    if command == "event" && parent.is_empty() {
        return Ok(());
    }
    let (sid, token) = if let (Ok(sid), Ok(token)) = (
        std::env::var("TERMINATOR_SESSION_ID"),
        std::env::var("TERMINATOR_SESSION_TOKEN"),
    ) {
        (sid, token)
    } else {
        // Some agents deliberately clear hook environments. Correlate only to an
        // actual live ancestor shell owned by this daemon, never by cwd or title.
        let Response::State(state) = rpc(&paths, Request::Snapshot)? else {
            bail!("No session inventory")
        };
        let matches: Vec<_> = state
            .sessions
            .iter()
            .filter(|s| s.lifecycle.live() && s.pid.is_some_and(|pid| ancestors.contains(&pid)))
            .collect();
        ensure!(
            matches.len() == 1,
            "Hook ancestry must identify exactly one owned session"
        );
        let session = matches
            .first()
            .copied()
            .context("Hook ancestry must identify exactly one owned session")?;
        paths = generations::owner_for(
            &paths,
            &Request::History {
                session: session.id.clone(),
            },
        )?;
        (session.id.clone(), paths.token()?)
    };
    let request = if command == "prompt" {
        if args.get(1).is_some_and(|s| s == "begin") {
            Request::ShellCommand { session: sid }
        } else {
            Request::ShellPrompt {
                session: sid,
                generation: args.get(1).context("Missing prompt generation")?.parse()?,
                jobs_empty: args.get(2).is_some_and(String::is_empty),
            }
        }
    } else if command == "cwd" {
        Request::Cwd {
            session: sid,
            path: args.get(1).context("Missing cwd")?.into(),
        }
    } else {
        // Bound stdin time as well as bytes: a misbehaving hook must not hang a CLI.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut data = Vec::new();
            let r = std::io::stdin().take(131073).read_to_end(&mut data);
            let _ = tx.send((r, data));
        });
        let (r, data) = rx.recv_timeout(Duration::from_millis(500))?;
        r?;
        if data.len() > 131072 {
            bail!("Payload too large")
        }
        if command == "emit" {
            let mut event: HookEvent = serde_json::from_slice(&data)?;
            event.terminal_session_id = sid;
            // Ancestor-supplied identity; the daemon verifies it against live
            // presence before linking lifecycle to a detected process.
            event.process = process;
            Request::Hook(event)
        } else {
            let kind = args.get(1).context("Missing adapter")?;
            let payload: serde_json::Value = serde_json::from_slice(&data)?;
            // Grok imports Claude hooks. Avoid treating those imported hooks as Claude events.
            if actual.as_deref().is_some_and(|a| a != kind) {
                return Ok(());
            }
            let Some(mut event) =
                terminator_integrations::normalize(kind, &sid, &parent, &payload)?
            else {
                return Ok(());
            };
            event.process = process;
            Request::Hook(event)
        }
    };
    let mut stream = connect(&paths, request, Some(token))?;
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    let response: Response = read_frame(&mut stream)?;
    if command == "prompt"
        && let Response::Text(generation) = response
    {
        println!("{generation}");
    }
    Ok(())
}
fn agent_parent() -> (
    String,
    Option<String>,
    Vec<u32>,
    Option<agents::ProcessIdentity>,
) {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut system = System::new();
    #[cfg(unix)]
    let mut pid = Pid::from_u32(std::os::unix::process::parent_id());
    #[cfg(not(unix))]
    let mut pid = {
        let mut system = System::new();
        let current = Pid::from_u32(std::process::id());
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[current]),
            false,
            ProcessRefreshKind::nothing(),
        );
        Pid::from_u32(
            system
                .process(current)
                .and_then(|process| process.parent())
                .map(|parent| parent.as_u32())
                .unwrap_or(0),
        )
    };
    let mut ancestors = Vec::new();
    let mut selected = None;
    for _ in 0..16 {
        if ancestors.contains(&pid.as_u32()) {
            break;
        }
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing().with_exe(UpdateKind::Always),
        );
        let Some(process) = system.process(pid) else {
            break;
        };
        ancestors.push(pid.as_u32());
        let executable = process
            .exe()
            .and_then(|p| p.file_name())
            .unwrap_or(process.name())
            .to_string_lossy();
        if selected.is_none()
            && !["sh", "bash", "zsh", "fish", "env", "terminator-hook"]
                .contains(&executable.as_ref())
        {
            let kind = terminator_integrations::AGENTS
                .iter()
                .find(|kind| executable == **kind || executable.starts_with(&format!("{kind}-bin")))
                .map(std::string::ToString::to_string);
            selected = Some((pid.as_u32(), process.start_time(), kind));
        }
        let Some(next) = process.parent().filter(|p| p.as_u32() > 1) else {
            break;
        };
        pid = next;
    }
    if let Some((pid, start_time, kind)) = selected {
        let mut cmd = std::process::Command::new("ps");
        cmd.args(["-p", &pid.to_string(), "-o", "lstart="]);
        if let Ok(out) = bounded_output(cmd, Duration::from_millis(500)) {
            let selected_pid = Pid::from_u32(pid);
            system.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[selected_pid]),
                true,
                ProcessRefreshKind::nothing(),
            );
            if out.status.success()
                && system
                    .process(selected_pid)
                    .is_some_and(|p| p.start_time() == start_time)
                && let Some(identity) = legacy_identity(pid, &out.stdout)
            {
                return (
                    identity,
                    kind,
                    ancestors,
                    Some(agents::ProcessIdentity { pid, start_time }),
                );
            }
        }
    }
    // No guessed PID-only ownership when process inspection is unavailable.
    (String::new(), None, ancestors, None)
}
fn legacy_identity(pid: u32, text: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(text).ok()?;
    let parts = text.split_whitespace().collect::<Vec<_>>();
    (parts.len() == 5).then(|| format!("{pid}:{}", parts.join("-")))
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
    let (rows, cols) = dimensions();
    write_frame(&mut *lock_writer(writer), &Request::Resize { rows, cols })
}
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
fn attach(session: &str, args: &[String]) -> Result<()> {
    #[cfg(unix)]
    let raw = rustix::termios::tcgetattr(std::io::stdin())
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
        });
    // Without a console (redirected stdio) there is no mode to set; the
    // bridge still works, but interactive programs may echo input. The
    // guard restores the previous console mode when dropped below.
    #[cfg(not(unix))]
    let raw = terminator_sys::RawMode::enable().ok();
    let paths = if args.len() >= 2 {
        Paths {
            data: args.first().context("Missing data path")?.clone().into(),
            runtime: args.get(1).context("Missing runtime path")?.clone().into(),
        }
    } else {
        Paths::discover()?
    };
    let (rows, cols) = dimensions();
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
    let input_writer = Arc::clone(&writer);
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
                &mut *lock_writer(&input_writer),
                &Request::Input {
                    data: B64.encode(chunk),
                },
            )
            .is_err()
            {
                break;
            }
        }
        let _ = lock_writer(&input_writer).shutdown();
    });
    #[cfg(unix)]
    {
        let mut signals = signal_hook::iterator::Signals::new([signal_hook::consts::SIGWINCH])?;
        let sigwinch_writer = Arc::clone(&writer);
        std::thread::spawn(move || {
            for _ in signals.forever() {
                if write_winsize(sigwinch_writer.as_ref()).is_err() {
                    break;
                }
            }
        });
    }
    // Windows has no SIGWINCH; poll the console size instead and forward
    // `Request::Resize` on change, like the signal handler does on Unix.
    #[cfg(windows)]
    {
        let resize_writer = Arc::clone(&writer);
        let mut known = (rows, cols);
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(200));
                if resize_changed(&mut known, dimensions())
                    && write_winsize(resize_writer.as_ref()).is_err()
                {
                    break;
                }
            }
        });
    }
    // Attach used the PTY size at spawn (often 80x50). Catch a GUI resize that
    // raced before this handler existed so COLUMNS matches the painted grid.
    let _ = write_winsize(writer.as_ref());
    let mut output = std::io::stdout();
    while let Ok(frame) = read_frame::<Response>(&mut stream) {
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
    drop(raw);
    Ok(())
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    #[test]
    fn legacy_identity_is_preserved_without_pid_only_fallback() {
        assert_eq!(
            legacy_identity(42, b"Tue Sep  8 10:11:12 2026\n").as_deref(),
            Some("42:Tue-Sep-8-10:11:12-2026")
        );
        assert!(legacy_identity(42, b"").is_none());
        assert!(legacy_identity(42, b"42").is_none());
    }
}

#[cfg(test)]
mod winsize_tests {
    use super::*;
    #[test]
    fn resizes_forward_once_per_size_change() {
        let mut known = (24, 80);
        assert!(!resize_changed(&mut known, (24, 80)));
        assert_eq!(known, (24, 80));
        assert!(resize_changed(&mut known, (30, 100)));
        assert_eq!(known, (30, 100));
        assert!(!resize_changed(&mut known, (30, 100)));
    }
    #[test]
    fn write_winsize_sends_a_resize_frame() {
        let buf = Mutex::new(Vec::new());
        assert!(write_winsize(&buf).is_ok());
        let bytes = buf.into_inner().unwrap_or_default();
        assert!(!bytes.is_empty());
        let req = read_frame::<Request>(&mut bytes.as_slice());
        assert!(
            matches!(req, Ok(Request::Resize { rows, cols }) if rows >= 1 && cols >= 1),
            "{req:?}"
        );
    }
}
