use super::pump_guarded;

/// Hands out one queued chunk per read.
struct Chunks(std::collections::VecDeque<&'static [u8]>);

impl std::io::Read for Chunks {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let Some(chunk) = self.0.pop_front() else {
            return Ok(0);
        };
        let n = chunk.len().min(buf.len());
        buf.get_mut(..n)
            .unwrap_or_default()
            .copy_from_slice(chunk.get(..n).unwrap_or_default());
        Ok(n)
    }
}

#[test]
fn a_parser_panic_is_reported_once_and_the_pty_keeps_draining() {
    let mut reader = Chunks([b"one".as_slice(), b"two", b"three"].into());
    let (mut reports, mut drained) = (0, Vec::new());
    pump_guarded(
        &mut reader,
        |data| assert_ne!(data, b"one".as_slice(), "parser bug"),
        || reports += 1,
        |data| drained.push(data.to_vec()),
    );
    let mut reader = Chunks([b"one".as_slice(), b"one"].into());
    let mut again = 0;
    pump_guarded(&mut reader, |_| panic!("parser bug"), || again += 1, |_| {});
    assert_eq!(reports, 1);
    assert_eq!(
        drained,
        [b"one".to_vec(), b"two".to_vec(), b"three".to_vec()]
    );
    assert_eq!(again, 1, "reported once, not per chunk");
}

/// Fails each read once with `Interrupted` before handing out a chunk.
struct Interrupting(Chunks, bool);

impl std::io::Read for Interrupting {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.1 = !self.1;
        if self.1 {
            return Err(std::io::ErrorKind::Interrupted.into());
        }
        self.0.read(buf)
    }
}

#[test]
fn an_interrupted_read_does_not_end_the_session_output() {
    let mut reader = Interrupting(Chunks([b"one".as_slice(), b"two"].into()), false);
    let mut drained = Vec::new();
    pump_guarded(
        &mut reader,
        |_| {},
        || {},
        |data| drained.push(data.to_vec()),
    );
    assert_eq!(drained, [b"one".to_vec(), b"two".to_vec()]);
}
