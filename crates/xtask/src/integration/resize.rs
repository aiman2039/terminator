//! Real-PTY resize: live resize, resize while detached, and alternate-screen replay.
use super::super::harness::{Harness, id};
use anyhow::Result;
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use serde_json::{Value, json};
use std::io::Write;
use terminator_core::{read_frame, transport, write_frame};

/// Attaches at a size and returns the stream with its first snapshot frame.
fn attach_at(
    h: &Harness,
    session: &Value,
    rows: u16,
    cols: u16,
) -> Result<(transport::Stream, Vec<u8>)> {
    let mut stream = h.connect(
        json!({"Attach":{"session":id(session),"rows":rows,"cols":cols}}),
        None,
        None,
    )?;
    let first: Value = read_frame(&mut stream)?;
    let snapshot = match first.get("Data").and_then(Value::as_str) {
        Some(data) => B64.decode(data)?,
        None => Vec::new(),
    };
    Ok((stream, snapshot))
}

fn contains(bytes: &[u8], needle: &str) -> bool {
    bytes
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

/// The shell reports the PTY size it sees.
fn expect_size(
    h: &Harness,
    session: &Value,
    stream: &mut transport::Stream,
    size: (u16, u16),
) -> Result<()> {
    let marker = format!("PTY_SIZE_{}_{}_OK", size.0, size.1);
    h.write(stream, "printf 'PTY_SIZE_%s_%s_OK\\n' $(stty size)\n")?;
    h.wait(
        |_| {
            h.history(id(session))
                .is_ok_and(|text| text.contains(&marker))
        },
        5,
    )?;
    Ok(())
}

pub(super) fn resize_and_replay(h: &Harness, session: &Value) -> Result<()> {
    let (mut stream, _) = attach_at(h, session, 10, 40)?;
    expect_size(h, session, &mut stream, (10, 40))?;
    write_frame(&mut stream, &json!({"Resize":{"rows":15,"cols":50}}))?;
    stream.flush()?;
    expect_size(h, session, &mut stream, (15, 50))?;
    drop(stream);
    // No client is attached; the next attach resizes the PTY.
    let (mut stream, _) = attach_at(h, session, 30, 120)?;
    expect_size(h, session, &mut stream, (30, 120))?;
    h.write(&mut stream, "printf '\\033[?1049hALT_%s_MARK' X\n")?;
    drop(stream);
    // Reattaching at another size replays the alternate screen.
    h.wait(
        |_| {
            attach_at(h, session, 12, 60).is_ok_and(|(_, snapshot)| {
                contains(&snapshot, "\x1b[?1049h") && contains(&snapshot, "ALT_X_MARK")
            })
        },
        5,
    )?;
    let (mut stream, _) = attach_at(h, session, 24, 80)?;
    h.write(&mut stream, "printf '\\033[?1049l'\n")?;
    drop(stream);
    h.wait(
        |_| {
            attach_at(h, session, 24, 80)
                .is_ok_and(|(_, snapshot)| !contains(&snapshot, "\x1b[?1049h"))
        },
        5,
    )?;
    let (mut stream, _) = attach_at(h, session, 24, 80)?;
    expect_size(h, session, &mut stream, (24, 80))
}
