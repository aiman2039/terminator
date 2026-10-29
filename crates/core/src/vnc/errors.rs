//! Why a VNC session failed, so viewers can offer the right fix instead of
//! only showing the message.
use serde::{Deserialize, Serialize};

/// Daemons that set [`crate::vnc::VncSession::error_kind`] advertise this.
pub const ERRORS_CAPABILITY: &str = "vnc-errors-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VncErrorKind {
    /// The VNC server rejected the password.
    AuthFailed,
    /// The server wants a password and none was given or saved.
    PasswordRequired,
    /// SSH refused the key or password.
    SshAuth,
    /// The SSH host key is unknown or changed.
    SshHostKey,
    /// The host or the VNC port cannot be reached.
    Unreachable,
    /// TLS setup or certificate validation failed.
    Tls,
    /// The configured start command failed on the SSH host.
    StartCommand,
    /// The server spoke unexpected VNC protocol.
    Protocol,
    /// Anything else.
    Other,
}

impl VncErrorKind {
    /// Map an OpenSSH failure (see [`crate::ssh::classify`]).
    #[must_use]
    pub fn from_ssh(failure: &crate::ssh::SshFailure) -> Self {
        use crate::ssh::SshFailure;
        match failure {
            SshFailure::UnknownHostKey | SshFailure::ChangedHostKey => Self::SshHostKey,
            SshFailure::AuthFailed => Self::SshAuth,
            SshFailure::Unreachable => Self::Unreachable,
            SshFailure::Other(_) => Self::Other,
        }
    }

    /// Classify a VNC handshake error message from the client library.
    #[must_use]
    pub fn from_handshake(message: &str) -> Self {
        let lower = message.to_lowercase();
        if lower.contains("password") || lower.contains("authentication failed") {
            Self::AuthFailed
        } else if lower.contains("tls") || lower.contains("certificate") {
            Self::Tls
        } else {
            Self::Protocol
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::SshFailure;

    #[test]
    fn ssh_and_handshake_failures_map_to_fixable_kinds() {
        assert_eq!(
            VncErrorKind::from_ssh(&SshFailure::AuthFailed),
            VncErrorKind::SshAuth
        );
        assert_eq!(
            VncErrorKind::from_ssh(&SshFailure::ChangedHostKey),
            VncErrorKind::SshHostKey
        );
        assert_eq!(
            VncErrorKind::from_handshake("Wrong password"),
            VncErrorKind::AuthFailed
        );
        assert_eq!(
            VncErrorKind::from_handshake("Invalid certificate"),
            VncErrorKind::Tls
        );
        assert_eq!(
            VncErrorKind::from_handshake("Unknown message"),
            VncErrorKind::Protocol
        );
    }
}
