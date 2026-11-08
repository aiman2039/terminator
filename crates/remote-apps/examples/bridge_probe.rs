//! End-to-end check of the host bridge, run on Linux by `dev/bridge-e2e.sh`
//! after it starts wprsd and an X11 app:
//!
//! `bridge_probe BRIDGE_BIN SOCKET|--ensure [--prime]`
//!
//! It acts as the local daemon: it starts the bridge, waits for a window and
//! its first frame, resizes the window, waits for a frame of the new size,
//! asks the window to close and waits for it to go.
//!
//! `--prime` only connects once and leaves. xwayland-xdg-shell maps X
//! windows only after wprsd has an output, and the bridge gives it one;
//! wprsd keeps it after the bridge leaves.

use std::io::{BufReader, BufWriter};
use std::process::{Command, ExitCode, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use terminator_remote_apps::protocol::{
    self, ClientMessage, Framed, HostMessage, PROTOCOL_VERSION, WindowId, WindowRole,
};

const WAIT: Duration = Duration::from_secs(15);

#[expect(clippy::print_stdout, reason = "The probe reports to the developer.")]
fn say(line: &str) {
    println!("{line}");
}

fn main() -> ExitCode {
    match probe() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            say(&format!("FAIL: {error}"));
            ExitCode::FAILURE
        }
    }
}

fn probe() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let (Some(bridge), Some(socket)) = (args.next(), args.next()) else {
        return Err("usage: bridge_probe BRIDGE_BIN SOCKET".into());
    };
    let bridge_args = if socket == "--ensure" {
        vec!["--ensure".to_owned()]
    } else {
        vec!["--socket".to_owned(), socket]
    };
    let mut child = Command::new(bridge)
        .args(bridge_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start bridge: {e}"))?;
    let stdout = child.stdout.take().ok_or("no bridge stdout")?;
    let mut stdin = BufWriter::new(child.stdin.take().ok_or("no bridge stdin")?);
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut input = BufReader::new(stdout);
        while let Ok(framed) = protocol::read_frame::<HostMessage>(&mut input) {
            if tx.send(framed).is_err() {
                break;
            }
        }
    });
    let mut send = |message: ClientMessage| {
        protocol::write_frame(&mut stdin, &message, &[]).map_err(|e| format!("send: {e}"))
    };
    send(ClientMessage::Hello {
        protocol: PROTOCOL_VERSION,
        version: "probe".into(),
        capabilities: Vec::new(),
    })?;
    if args.next().as_deref() == Some("--prime") {
        wait_for(&rx, "hello", |m| {
            matches!(m.message, HostMessage::Hello { .. }).then_some(())
        })?;
        thread::sleep(Duration::from_secs(1));
        drop(child.stdin.take());
        let _ = child.kill();
        say("primed: wprsd has an output");
        return Ok(());
    }

    let id = wait_for(&rx, "a toplevel window", |m| match &m.message {
        HostMessage::WindowOpened {
            id,
            role: WindowRole::Toplevel,
            title,
            ..
        } => {
            say(&format!("window {} opened: {title:?}", id.0));
            Some(*id)
        }
        _ => None,
    })?;
    let first = wait_for(&rx, "the first frame", |m| frame_size(m, id))?;
    say(&format!("first frame {}x{}", first.0, first.1));

    send(ClientMessage::Configure {
        id,
        width: 400,
        height: 300,
        scale: 1,
        focused: true,
    })?;
    let resized = wait_for(&rx, "a 400x300 frame", |m| {
        frame_size(m, id).filter(|size| *size == (400, 300))
    })?;
    say(&format!("resized frame {}x{}", resized.0, resized.1));

    send(ClientMessage::CloseRequested { id })?;
    wait_for(&rx, "the window to close", |m| match m.message {
        HostMessage::WindowClosed { id: closed } if closed == id => Some(()),
        _ => None,
    })?;
    drop(child.stdin.take());
    let _ = child.kill();
    say("PASS: bridge opened, resized and closed the window");
    Ok(())
}

fn frame_size(message: &Framed<HostMessage>, want: WindowId) -> Option<(u32, u32)> {
    match message.message {
        HostMessage::Frame {
            id, width, height, ..
        } if id == want => {
            let expected = u64::from(width)
                .checked_mul(u64::from(height))?
                .checked_mul(4)?;
            (zstd_len(&message.payload)? == expected).then_some((width, height))
        }
        _ => None,
    }
}

fn zstd_len(payload: &[u8]) -> Option<u64> {
    zstd::bulk::decompress(payload, 64 << 20)
        .ok()
        .and_then(|bytes| u64::try_from(bytes.len()).ok())
}

fn wait_for<T>(
    rx: &mpsc::Receiver<Framed<HostMessage>>,
    what: &str,
    mut matches: impl FnMut(&Framed<HostMessage>) -> Option<T>,
) -> Result<T, String> {
    loop {
        let framed = rx
            .recv_timeout(WAIT)
            .map_err(|_| format!("timed out waiting for {what}"))?;
        say(&format!("  < {:?}", framed.message));
        if let HostMessage::Error { kind, message } = &framed.message {
            return Err(format!("bridge error {kind:?}: {message}"));
        }
        if let Some(found) = matches(&framed) {
            return Ok(found);
        }
    }
}
