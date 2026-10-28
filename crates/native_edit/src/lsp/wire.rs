use super::*;

/// JSON-RPC request id: integer or string, echoed back by the server.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RequestId {
    Number(i64),
    Text(String),
}

/// Monotonic request ids; wraps instead of overflowing.
#[derive(Default, Debug)]
pub struct IdGen {
    next: i64,
}

impl IdGen {
    /// Fresh id for one outgoing request.
    pub fn next_id(&mut self) -> RequestId {
        let id = self.next;
        self.next = self.next.wrapping_add(1);
        RequestId::Number(id)
    }
}

/// One decoded JSON-RPC message.
#[derive(Clone, PartialEq, Debug)]
pub enum LspMessage {
    Request {
        id: RequestId,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
    Response {
        id: RequestId,
        result: Option<Value>,
        error: Option<ResponseError>,
    },
}

/// JSON-RPC error payload on a response.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ResponseError {
    pub code: i64,
    pub message: String,
}

/// Frame one JSON body with its `Content-Length` header.
#[must_use]
pub fn frame(body: &[u8]) -> Vec<u8> {
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(body);
    out
}

/// Incremental `Content-Length` framing decoder: feed arbitrary stdout
/// chunks, drain complete bodies. Unknown headers are skipped; a header
/// block without `Content-Length` is discarded through its terminator.
#[derive(Default, Debug)]
pub struct FrameDecoder {
    buf: Vec<u8>,
}

impl FrameDecoder {
    /// Feed bytes; returns every complete message body now available.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        loop {
            match take_frame(&mut self.buf) {
                Frame::Body(body) => out.push(body),
                // Drained a header without a body claim: resync and keep
                // scanning; every drain makes progress, so this ends.
                Frame::Drained => continue,
                Frame::Incomplete => break,
            }
        }
        out
    }
}

pub(super) enum Frame {
    Body(Vec<u8>),
    Drained,
    Incomplete,
}

/// Split one complete frame off the front of `buf`, if present. A
/// header block without `Content-Length` is discarded so the stream
/// resyncs on the next frame instead of stalling.
pub(super) fn take_frame(buf: &mut Vec<u8>) -> Frame {
    let Some(end) = find_header_end(buf) else {
        return Frame::Incomplete;
    };
    // Only scan the header block; the body may contain anything.
    let header = buf.get(..end).unwrap_or_default().to_vec();
    let mut length: Option<usize> = None;
    for line in header.split(|b| *b == b'\n') {
        if let Some(value) = line
            .strip_prefix(b"Content-Length:")
            .or_else(|| line.strip_prefix(b"content-length:"))
        {
            let text = std::str::from_utf8(value).unwrap_or("").trim();
            length = text.parse::<usize>().ok();
        }
    }
    let Some(length) = length else {
        // Explicit consume: `drain` is lazy, dropping it unfinished still
        // removes the range, but spelling the consume out leaves no doubt.
        buf.drain(..end).for_each(drop);
        return Frame::Drained;
    };
    let total = end.saturating_add(length);
    if buf.len() < total {
        return Frame::Incomplete;
    }
    // Drop the header first: draining `end..total` would leave it
    // prepended and stall every later frame on a bogus huge length.
    buf.drain(..end).for_each(drop);
    Frame::Body(buf.drain(..length).collect())
}

/// Byte offset just past the `\r\n\r\n` terminating the header block.
pub(super) fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|at| at.saturating_add(4))
}

/// Classify one complete message body. `None` means not valid
/// JSON-RPC (the caller logs and drops it).
#[must_use]
pub fn parse_message(body: &[u8]) -> Option<LspMessage> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let object = value.as_object()?;
    let id = object.get("id").and_then(parse_id);
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .map(String::from);
    let params = object.get("params").cloned().unwrap_or(Value::Null);
    match (id, method) {
        (Some(id), Some(method)) => Some(LspMessage::Request { id, method, params }),
        (None, Some(method)) => Some(LspMessage::Notification { method, params }),
        (Some(id), None) => {
            let result = object.get("result").cloned();
            let error = object.get("error").and_then(parse_error);
            Some(LspMessage::Response { id, result, error })
        }
        (None, None) => None,
    }
}

pub(super) fn parse_id(value: &Value) -> Option<RequestId> {
    match value {
        Value::Number(number) => number.as_i64().map(RequestId::Number),
        Value::String(text) => Some(RequestId::Text(text.clone())),
        _ => None,
    }
}

pub(super) fn parse_error(value: &Value) -> Option<ResponseError> {
    let object = value.as_object()?;
    Some(ResponseError {
        code: object.get("code")?.as_i64()?,
        message: object
            .get("message")
            .and_then(Value::as_str)
            .map_or(String::new(), String::from),
    })
}

pub(super) fn id_json(id: &RequestId) -> Value {
    match id {
        RequestId::Number(number) => Value::from(*number),
        RequestId::Text(text) => Value::from(text.clone()),
    }
}

pub(super) fn request(id: &RequestId, method: &str, params: Value) -> Vec<u8> {
    frame(
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": id_json(id),
            "method": method,
            "params": params,
        })
        .to_string()
        .as_bytes(),
    )
}

pub(super) fn notification(method: &str, params: Value) -> Vec<u8> {
    frame(
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        })
        .to_string()
        .as_bytes(),
    )
}
