//! Classified OpenSSH failures, as daemons report them to clients.
use super::*;

/// Why an OpenSSH call failed, derived from its stderr.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SshFailure {
    UnknownHostKey,
    ChangedHostKey,
    AuthFailed,
    Unreachable,
    Other(String),
}

/// Classifies OpenSSH client stderr. A changed key is checked first: its
/// warning also ends with "Host key verification failed.".
#[must_use]
pub fn classify(stderr: &str) -> SshFailure {
    let has = |needle: &str| stderr.contains(needle);
    if has("REMOTE HOST IDENTIFICATION HAS CHANGED")
        || has("has changed and you have requested strict checking")
    {
        SshFailure::ChangedHostKey
    } else if has("Host key verification failed.") || has(" host key is known for ") {
        SshFailure::UnknownHostKey
    } else if has("Permission denied (") || has("Too many authentication failures") {
        SshFailure::AuthFailed
    } else if [
        "Connection refused",
        "Could not resolve hostname",
        "Connection timed out",
        "Operation timed out",
        "No route to host",
        "Network is unreachable",
    ]
    .iter()
    .any(|needle| has(needle))
    {
        SshFailure::Unreachable
    } else {
        SshFailure::Other(stderr.trim().to_owned())
    }
}

/// A classified OpenSSH failure. Daemons send it as `Response::SshError`
/// to clients that opt in; `Response::checked` turns it back into this type
/// so callers can find it with [`SshError::find`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshError {
    pub failure: SshFailure,
    pub message: String,
}
impl std::fmt::Display for SshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for SshError {}
impl SshError {
    /// The classified failure anywhere in `error`'s source chain, including
    /// one a core error enum forwards transparently.
    #[must_use]
    pub fn find<'a>(error: &'a (dyn std::error::Error + 'static)) -> Option<&'a Self> {
        std::iter::successors(Some(error), |e| e.source()).find_map(|e| {
            if let Some(ssh) = e.downcast_ref::<Self>() {
                return Some(ssh);
            }
            if let Some(ipc) = e.downcast_ref::<crate::IpcError>() {
                return ipc.ssh();
            }
            #[cfg(feature = "async-client")]
            if let Some(client) = e.downcast_ref::<crate::async_client::AsyncClientError>() {
                return client.ssh();
            }
            None
        })
    }
    /// `None` when stderr is not a recognized SSH connection failure.
    #[must_use]
    pub fn from_stderr(stderr: &[u8]) -> Option<Self> {
        let text = String::from_utf8_lossy(stderr);
        let failure = classify(&text);
        let summary = match failure {
            SshFailure::UnknownHostKey => "The SSH host key is not trusted yet",
            SshFailure::ChangedHostKey => "The SSH host key changed",
            SshFailure::AuthFailed => "SSH authentication failed",
            SshFailure::Unreachable => "The SSH host is unreachable",
            SshFailure::Other(_) => return None,
        };
        Some(Self {
            failure,
            message: format!("{summary}: {}", text.trim()),
        })
    }
}
