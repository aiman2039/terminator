//! One request to one owner's socket: encoding, deadlines, and chunked responses.
use super::*;
use crate::{
    Envelope, FrameError, MAX_FRAME, PROTOCOL_VERSION, SnapshotHint,
    snapshot::{self, SnapshotError},
    transport::AsyncStream,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

impl Client {
    pub async fn direct(
        &self,
        paths: Paths,
        request: Request,
        hint: Option<SnapshotHint>,
    ) -> Result<Response> {
        let token_paths = paths.clone();
        let token = self
            .catalog
            .run(&CancellationToken::new(), move || {
                token_paths.token().map_err(AsyncClientError::from)
            })
            .await?;
        let mutation = !read_only(&request);
        // Static operation name for the deadline below; the envelope takes
        // ownership of the request afterwards.
        let operation = request.name();
        let timeout = request.response_deadline();
        let envelope = Envelope {
            version: PROTOCOL_VERSION,
            auth: token,
            snapshot_chunks: matches!(request, Request::Snapshot | Request::VncFrame { .. }),
            ssh_errors: true,
            request,
            snapshot_hint: hint,
        };
        let bytes = self.encode(envelope).await?;
        let socket = paths.socket();
        let mut stream = tokio::time::timeout(timeout, AsyncStream::connect_ipc(&socket))
            .await
            .map_err(|source| AsyncClientError::ConnectDeadline { socket, source })?
            .map_err(|source| AsyncClientError::Unavailable { source })?;
        let result = match tokio::time::timeout(timeout, self.exchange(&mut stream, &bytes)).await {
            Ok(result) => result,
            Err(source) => Err(AsyncClientError::ResponseDeadline { operation, source }),
        };
        if mutation {
            result.map_err(|error| AsyncClientError::UncertainMutation {
                source: Box::new(error),
            })
        } else {
            result
        }
    }

    async fn encode(&self, envelope: Envelope) -> Result<Vec<u8>> {
        let inline = inline_envelope(&envelope);
        let encode = move || -> Result<Vec<u8>> {
            let bytes = serde_json::to_vec(&envelope)?;
            if bytes.len() > MAX_FRAME {
                return Err(FrameError::TooLarge { bytes: bytes.len() }.into());
            }
            Ok(bytes)
        };
        // Tiny protocol messages must not queue behind noninterruptible image/diff work.
        if inline {
            encode()
        } else {
            self.cpu.run(&CancellationToken::new(), encode).await
        }
    }

    async fn exchange(&self, socket: &mut AsyncStream, bytes: &[u8]) -> Result<Response> {
        let len =
            u32::try_from(bytes.len()).map_err(|_| FrameError::TooLarge { bytes: bytes.len() })?;
        socket.write_all(&len.to_be_bytes()).await?;
        socket.write_all(bytes).await?;
        let mut response = self.read_response(socket).await?;
        if let Response::State(state) = &mut response {
            self.observe(state);
        }
        Ok(response)
    }

    async fn frame(&self, socket: &mut AsyncStream) -> Result<Response> {
        let n = socket.read_u32().await? as usize;
        if n > MAX_FRAME {
            return Err(FrameError::TooLarge { bytes: n }.into());
        }
        let mut bytes = vec![0; n];
        socket.read_exact(&mut bytes).await?;
        if bytes.len() <= 64 * 1024 {
            return Ok(serde_json::from_slice(&bytes)?);
        }
        self.cpu
            .run(&CancellationToken::new(), move || {
                Ok(serde_json::from_slice(&bytes)?)
            })
            .await
    }

    async fn read_response(&self, socket: &mut AsyncStream) -> Result<Response> {
        let mut response = self.frame(socket).await?;
        if !matches!(response, Response::SnapshotChunk { .. }) {
            return Ok(response);
        }
        let mut bytes = Vec::new();
        loop {
            let Response::SnapshotChunk { data, last } = response else {
                return Err(SnapshotError::Interrupted.into());
            };
            let chunk = self
                .cpu
                .run(&CancellationToken::new(), move || {
                    snapshot::decode(&data).map_err(AsyncClientError::from)
                })
                .await?;
            if !bytes
                .len()
                .checked_add(chunk.len())
                .is_some_and(|total| total <= snapshot::MAX_RESPONSE_BYTES)
            {
                return Err(AsyncClientError::ResponseTooLarge { bytes: bytes.len() });
            }
            bytes.extend(chunk);
            if last {
                break;
            }
            response = self.frame(socket).await?;
        }
        self.cpu
            .run(&CancellationToken::new(), move || {
                let response = serde_json::from_slice(&bytes)?;
                if !matches!(response, Response::State(_) | Response::VncFrame(_)) {
                    return Err(SnapshotError::UnexpectedResponse.into());
                }
                Ok(response)
            })
            .await
    }
}

/// Small protocol messages encode on the calling task instead of the CPU pool.
fn inline_envelope(envelope: &Envelope) -> bool {
    envelope.auth.len() <= 4096
        && match &envelope.request {
            Request::Snapshot | Request::Heartbeat { .. } => true,
            Request::Focus { session }
            | Request::Stop { session }
            | Request::EditorSave { session }
            | Request::EditorStatus { session }
            | Request::Screen { session }
            | Request::History { session }
            | Request::VncFrame { session, .. }
            | Request::VncDisconnect { session } => session.len() <= 128,
            Request::VncInput { session, events } => session.len() <= 128 && events.len() <= 32,
            _ => false,
        }
}
