//! Opt-in snapshot streaming keeps the per-frame limit without dropping state.
use crate::{MAX_FRAME, Response, read_frame, write_frame};
use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use std::io::{self, Cursor, Read, Write};

pub const CAPABILITY: &str = "snapshot-chunks-v1";
const CHUNK_BYTES: usize = 512 * 1024;

pub fn write_response(writer: &mut impl Write, response: &Response, chunks: bool) -> Result<()> {
    let bytes = serde_json::to_vec(response)?;
    if bytes.len() <= MAX_FRAME {
        writer.write_all(&(bytes.len() as u32).to_be_bytes())?;
        writer.write_all(&bytes)?;
        writer.flush()?;
    } else if chunks && matches!(response, Response::State(_)) {
        for (index, data) in bytes.chunks(CHUNK_BYTES).enumerate() {
            write_frame(
                writer,
                &Response::SnapshotChunk {
                    data: B64.encode(data),
                    last: (index + 1) * CHUNK_BYTES >= bytes.len(),
                },
            )?;
        }
    } else {
        let message = if matches!(response, Response::State(_)) {
            "Response exceeds the legacy IPC limit; update the GUI or terminator-hook to receive large snapshots. Saved state is intact."
        } else {
            "Response exceeds the IPC frame limit."
        };
        write_frame(writer, &Response::Error(message.into()))?;
    }
    Ok(())
}

pub fn read_response(reader: &mut impl Read) -> Result<Response> {
    let response = read_frame(reader)?;
    let Response::SnapshotChunk { data, last } = response else {
        return Ok(response);
    };
    // Deserialize incrementally instead of allocating another full snapshot.
    let stream = Chunks {
        reader,
        current: Cursor::new(decode(&data)?),
        last,
    };
    let response = serde_json::from_reader(stream)?;
    ensure!(
        matches!(response, Response::State(_)),
        "Expected snapshot state"
    );
    Ok(response)
}

pub(crate) fn decode(data: &str) -> Result<Vec<u8>> {
    ensure!(
        data.len() <= CHUNK_BYTES.div_ceil(3) * 4,
        "Snapshot chunk too large"
    );
    let bytes = B64.decode(data)?;
    ensure!(
        !bytes.is_empty() && bytes.len() <= CHUNK_BYTES,
        "Invalid snapshot chunk"
    );
    Ok(bytes)
}
struct Chunks<'a, R> {
    reader: &'a mut R,
    current: Cursor<Vec<u8>>,
    last: bool,
}
impl<R: Read> Read for Chunks<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            let n = self.current.read(buf)?;
            if n != 0 || self.last {
                return Ok(n);
            }
            let next = (|| -> Result<()> {
                let Response::SnapshotChunk { data, last } = read_frame(self.reader)? else {
                    anyhow::bail!("Interrupted snapshot transfer");
                };
                self.current = Cursor::new(decode(&data)?);
                self.last = last;
                Ok(())
            })();
            next.map_err(io::Error::other)?;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::State;
    #[test]
    fn large_snapshot_round_trips_without_exceeding_any_frame_limit() {
        let state = State {
            degraded: Some("界\n\"".repeat(MAX_FRAME / 4)),
            ..State::default()
        };
        let mut wire = vec![];
        write_response(&mut wire, &Response::State(Box::new(state.clone())), true).unwrap();
        let mut frames = wire.as_slice();
        let mut count = 0;
        while !frames.is_empty() {
            assert!(matches!(
                read_frame::<Response>(&mut frames).unwrap(),
                Response::SnapshotChunk { .. }
            ));
            count += 1;
        }
        assert!(count > 1);
        let Response::State(restored) = read_response(&mut wire.as_slice()).unwrap() else {
            panic!("Expected state")
        };
        assert_eq!(restored.degraded, state.degraded);
        assert!(read_response(&mut &wire[..wire.len() - 1]).is_err());
    }
    #[test]
    fn legacy_clients_receive_normal_frames_or_an_actionable_error() {
        for chunks in [false, true] {
            let mut wire = vec![];
            write_response(&mut wire, &Response::State(Box::default()), chunks).unwrap();
            assert!(matches!(
                read_frame::<Response>(&mut wire.as_slice()).unwrap(),
                Response::State(_)
            ));
        }
        let mut wire = vec![];
        write_response(
            &mut wire,
            &Response::State(Box::new(State {
                degraded: Some("x".repeat(MAX_FRAME)),
                ..State::default()
            })),
            false,
        )
        .unwrap();
        let Response::Error(error) = read_frame::<Response>(&mut wire.as_slice()).unwrap() else {
            panic!("Expected legacy size error")
        };
        assert!(error.contains("update"));
    }
    #[test]
    fn malformed_or_interrupted_chunk_streams_fail_explicitly() {
        for data in ["%%%".to_owned(), B64.encode("{\"State\":"), String::new()] {
            let mut wire = vec![];
            write_frame(&mut wire, &Response::SnapshotChunk { data, last: false }).unwrap();
            write_frame(&mut wire, &Response::Ok).unwrap();
            assert!(read_response(&mut wire.as_slice()).is_err());
        }
    }
}
