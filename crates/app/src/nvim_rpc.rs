//! Bounded local `MessagePack` requests to an existing Neovim process.
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::Value;
use std::{io, path::Path, time::Duration};
#[cfg(test)]
#[cfg(unix)]
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    time::Instant,
};

#[cfg(test)]
#[cfg(unix)]
pub struct Connection {
    stream: UnixStream,
    deadline: Instant,
    next: u64,
}
#[cfg(test)]
#[cfg(unix)]
impl Connection {
    pub fn connect(path: &Path, timeout: Duration) -> Result<Self> {
        Ok(Self {
            stream: UnixStream::connect(path).context("Connect to Neovim")?,
            deadline: Instant::now()
                .checked_add(timeout)
                .context("Neovim deadline overflow")?,
            next: 0,
        })
    }
    pub fn call(&mut self, method: &str, args: Value, limit: usize) -> Result<Value> {
        self.next = self
            .next
            .checked_add(1)
            .context("Neovim request id overflow")?;
        self.stream
            .set_write_timeout(Some(remaining(self.deadline)?))?;
        self.stream
            .write_all(&rmp_serde::to_vec(&(0, self.next, method, args))?)?;
        let reader = Limited {
            stream: &self.stream,
            deadline: self.deadline,
            remaining: limit,
        };
        let mut decoder = rmp_serde::Deserializer::new(reader);
        decoder.set_max_depth(32);
        let response = Value::deserialize(&mut decoder).context("Read Neovim response")?;
        let parts = response.as_array().context("Invalid Neovim response")?;
        ensure!(
            parts.len() == 4 && parts[0] == 1 && parts[1] == self.next,
            "Unexpected Neovim response identity"
        );
        ensure!(parts[2].is_null(), "Neovim request failed: {}", parts[2]);
        Ok(parts[3].clone())
    }
}
#[cfg(test)]
#[cfg(unix)]
fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "Neovim request deadline exceeded"))
}
#[cfg(test)]
#[cfg(unix)]
struct Limited<'a> {
    stream: &'a UnixStream,
    deadline: Instant,
    remaining: usize,
}
#[cfg(test)]
#[cfg(unix)]
impl Read for Limited<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Neovim response exceeds preview limit",
            ));
        }
        // poll enforces one deadline for the complete response, including slow
        // partial frames. It also permits draining a peer that already closed;
        // updating SO_RCVTIMEO on such a socket can fail with EINVAL on macOS.
        loop {
            let cap = u128::try_from(i32::MAX).unwrap_or(1);
            let wait = i32::try_from(remaining(self.deadline)?.as_millis().clamp(1, cap))
                .unwrap_or(i32::MAX);
            let timeout = rustix::event::Timespec::try_from(Duration::from_millis(
                u64::try_from(wait).unwrap_or(0),
            ))
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
            let mut descriptor = [rustix::event::PollFd::new(
                &self.stream,
                rustix::event::PollFlags::IN,
            )];
            match rustix::event::poll(&mut descriptor, Some(&timeout)) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "Neovim request deadline exceeded",
                    ));
                }
                Ok(_) => break,
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        let limit = bytes.len().min(self.remaining);
        let Some(buf) = bytes.get_mut(..limit) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Neovim read buffer is short",
            ));
        };
        let read = self.stream.read(buf)?;
        self.remaining = self.remaining.checked_sub(read).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "Neovim read exceeded the limit")
        })?;
        Ok(read)
    }
}
pub fn timed_out(error: &anyhow::Error) -> bool {
    error.chain().any(|error| {
        error.downcast_ref::<io::Error>().is_some_and(|e| {
            matches!(
                e.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            )
        })
    })
}

pub struct AsyncConnection {
    #[cfg(unix)]
    stream: tokio::net::UnixStream,
    #[cfg(windows)]
    stream: tokio::net::windows::named_pipe::NamedPipeClient,
    deadline: tokio::time::Instant,
    next: u64,
    cpu: terminator_core::async_service::NativePool,
}
impl AsyncConnection {
    pub async fn connect(
        path: &Path,
        timeout: Duration,
        cpu: terminator_core::async_service::NativePool,
    ) -> Result<Self> {
        let deadline = tokio::time::Instant::now()
            .checked_add(timeout)
            .context("Neovim connect deadline overflow")?;
        #[cfg(unix)]
        let stream = tokio::time::timeout_at(deadline, tokio::net::UnixStream::connect(path))
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Neovim connection deadline exceeded",
                )
            })??;
        #[cfg(windows)]
        let stream = {
            // `ClientOptions::open` is synchronous: retry fast failures (no
            // listener yet) until the same deadline the Unix await enforces.
            let pipe = terminator_core::transport::nvim_listen_arg(path);
            loop {
                match tokio::net::windows::named_pipe::ClientOptions::new().open(&pipe) {
                    Ok(stream) => break stream,
                    Err(_) if tokio::time::Instant::now() < deadline => {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    Err(_) => {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "Neovim connection deadline exceeded",
                        )
                        .into());
                    }
                }
            }
        };
        Ok(Self {
            stream,
            deadline,
            next: 0,
            cpu,
        })
    }
    pub async fn call(&mut self, method: &str, args: Value, limit: usize) -> Result<Value> {
        use terminator_core::async_service::CancellationToken;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        self.next = self
            .next
            .checked_add(1)
            .context("Neovim request id overflow")?;
        let id = self.next;
        let deadline = self.deadline;
        tokio::time::timeout_at(deadline, async {
            self.stream
                .write_all(&rmp_serde::to_vec(&(0, id, method, args))?)
                .await?;
            let mut bytes = Vec::new();
            loop {
                ensure!(bytes.len() < limit, "Neovim response exceeds preview limit");
                let mut chunk = [0; 8192];
                let Some(room) = limit.checked_sub(bytes.len()) else {
                    anyhow::bail!("Neovim response exceeds preview limit");
                };
                let available = chunk.len().min(room);
                let Some(buf) = chunk.get_mut(..available) else {
                    anyhow::bail!("Neovim read buffer is short");
                };
                let n = self.stream.read(buf).await?;
                ensure!(n > 0, "Neovim connection closed before response");
                let Some(read) = chunk.get(..n) else {
                    anyhow::bail!("Neovim read exceeded the buffer");
                };
                bytes.extend_from_slice(read);
                let parse = move || {
                    let mut decoder = rmp_serde::Deserializer::from_read_ref(&bytes);
                    decoder.set_max_depth(32);
                    let parsed = Value::deserialize(&mut decoder);
                    Ok((bytes, parsed))
                };
                let (buffer, parsed) = if n <= 4096 && limit <= 4096 {
                    parse()?
                } else {
                    self.cpu.run(&CancellationToken::new(), parse).await?
                };
                bytes = buffer;
                let mut response = match parsed {
                    Ok(value) => value,
                    Err(
                        rmp_serde::decode::Error::InvalidMarkerRead(ref e)
                        | rmp_serde::decode::Error::InvalidDataRead(ref e),
                    ) if e.kind() == io::ErrorKind::UnexpectedEof => continue,
                    Err(error) => return Err(error.into()),
                };
                let parts = response.as_array().context("Invalid Neovim response")?;
                ensure!(
                    parts.len() == 4
                        && parts.first().is_some_and(|part| *part == 1)
                        && parts.get(1).is_some_and(|part| *part == id),
                    "Unexpected Neovim response identity"
                );
                let err = parts.get(2).context("Neovim response missing error")?;
                ensure!(err.is_null(), "Neovim request failed: {err}");
                return response
                    .as_array_mut()
                    .context("Invalid Neovim response")?
                    .pop()
                    .context("Neovim response missing result");
            }
        })
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Neovim request deadline exceeded"))?
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    #[test]
    fn mismatched_rpc_response_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rpc.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (mut peer, _) = listener.accept().unwrap();
            let _: Value = rmp_serde::from_read(&mut peer).unwrap();
            peer.write_all(&rmp_serde::to_vec(&(1, 99, Value::Null, true)).unwrap())
                .unwrap();
        });
        let mut rpc = Connection::connect(&path, Duration::from_secs(1)).unwrap();
        let error = rpc
            .call("nvim_get_mode", serde_json::json!([]), 4096)
            .unwrap_err();
        assert!(error.to_string().contains("identity"), "{error:#}");
        server.join().unwrap();
    }

    #[test]
    fn socket_deadline_and_response_size_are_bounded() {
        for overflow in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("rpc.sock");
            let listener = UnixListener::bind(&path).unwrap();
            let server = std::thread::spawn(move || {
                let (mut peer, _) = listener.accept().unwrap();
                let _: Value = rmp_serde::from_read(&mut peer).unwrap();
                if overflow {
                    let _ = peer.write_all(
                        &rmp_serde::to_vec(&(1, 1, Value::Null, "x".repeat(2048))).unwrap(),
                    );
                } else {
                    std::thread::sleep(Duration::from_millis(150));
                }
            });
            let started = Instant::now();
            let mut rpc = Connection::connect(&path, Duration::from_millis(50)).unwrap();
            let error = rpc
                .call("nvim_get_mode", serde_json::json!([]), 128)
                .unwrap_err();
            if overflow {
                assert!(format!("{error:#}").contains("limit"), "{error:#}");
            } else {
                assert!(timed_out(&error), "{error:#}");
            }
            assert!(started.elapsed() < Duration::from_secs(1));
            server.join().unwrap();
        }
    }
}
