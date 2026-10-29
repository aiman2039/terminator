use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const FEATURES_CAPABILITY: &str = "vnc-features-v1";
/// Daemons that run [`VncOptions::ssh_start_command`] before connecting.
pub const START_COMMAND_CAPABILITY: &str = "vnc-start-command-v1";
pub const MAX_START_COMMAND: usize = 1024;
pub const MAX_CLIPBOARD: usize = 256 * 1024;
pub const MAX_TRANSFER_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    #[default]
    Direct,
    Tls,
    Ssh,
}

pub use crate::ssh::{SshOptions, validate_host};

/// Why VNC connection options are invalid.
#[derive(Debug, thiserror::Error)]
pub enum VncOptionsError {
    #[error("Reconnect attempts must not exceed 10")]
    TooManyReconnectAttempts { attempts: u8 },
    #[error("Reconnect requires at least one attempt")]
    NoReconnectAttempts,
    #[error("Invalid TLS server name")]
    InvalidTlsServerName { name: String },
    #[error("TLS CA file must be an absolute path")]
    RelativeTlsCaFile { path: PathBuf },
    #[error(transparent)]
    Ssh(#[from] crate::ssh::SshOptionsError),
    #[error("The start command must be one line of at most 1024 characters")]
    InvalidStartCommand,
    #[error("A start command requires the SSH tunnel transport")]
    StartCommandWithoutSsh,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    #[default]
    Auto,
    LowBandwidth,
    Sharp,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardMode {
    Disabled,
    ReceiveOnly,
    SendOnly,
    #[default]
    Both,
}
impl ClipboardMode {
    pub fn send(self) -> bool {
        matches!(self, Self::SendOnly | Self::Both)
    }
    pub fn receive(self) -> bool {
        matches!(self, Self::ReceiveOnly | Self::Both)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReconnectOptions {
    pub enabled: bool,
    pub max_attempts: u8,
}
impl Default for ReconnectOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            max_attempts: 5,
        }
    }
}
impl ReconnectOptions {
    pub fn delay_seconds(&self, attempt: u8) -> u64 {
        1u64.checked_shl(u32::from(attempt.min(5)))
            .unwrap_or(30)
            .min(30)
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VncOptions {
    pub transport: Transport,
    pub allow_no_auth: bool,
    pub tls_server_name: String,
    pub tls_ca_file: Option<PathBuf>,
    pub ssh: SshOptions,
    pub reconnect: ReconnectOptions,
    pub clipboard: ClipboardMode,
    pub quality: Quality,
    pub remote_resize: bool,
    pub show_remote_cursor: bool,
    /// Shell command run on the SSH host before each connection, for
    /// example to start the VNC server. Empty runs nothing. Omitted when
    /// empty so older daemons see unchanged options.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub ssh_start_command: String,
    /// What the Mac Command key sends. Viewer-only; the daemon ignores it.
    #[serde(skip_serializing_if = "CommandKey::is_default")]
    pub command_key: CommandKey,
}

/// The remote key that the local Mac Command key presses. Elsewhere the
/// local Control key already sends Control.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandKey {
    /// Cmd+A, Cmd+Z, Cmd+S act as Ctrl chords, as Linux and Windows expect.
    #[default]
    Control,
    /// The Super (Windows) key.
    Super,
}

impl CommandKey {
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::Control
    }
}
impl VncOptions {
    pub fn validate(&self) -> Result<(), VncOptionsError> {
        if self.reconnect.max_attempts > 10 {
            return Err(VncOptionsError::TooManyReconnectAttempts {
                attempts: self.reconnect.max_attempts,
            });
        }
        if self.reconnect.enabled && self.reconnect.max_attempts == 0 {
            return Err(VncOptionsError::NoReconnectAttempts);
        }
        if self.tls_server_name.len() > 255 || self.tls_server_name.chars().any(char::is_control) {
            return Err(VncOptionsError::InvalidTlsServerName {
                name: self.tls_server_name.clone(),
            });
        }
        if let Some(path) = &self.tls_ca_file
            && !path.is_absolute()
        {
            return Err(VncOptionsError::RelativeTlsCaFile { path: path.clone() });
        }
        if self.transport == Transport::Ssh {
            self.ssh.validate()?;
        }
        if !self.ssh_start_command.is_empty() {
            if self.ssh_start_command.len() > MAX_START_COMMAND
                || self.ssh_start_command.chars().any(char::is_control)
            {
                return Err(VncOptionsError::InvalidStartCommand);
            }
            if self.transport != Transport::Ssh {
                return Err(VncOptionsError::StartCommandWithoutSsh);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ssh_rejects_option_injection_and_invalid_paths() {
        let mut options = SshOptions::default();
        for host in [
            "-oProxyCommand=bad",
            "host name",
            "host\nother",
            "",
            "user@host",
        ] {
            options.host = host.into();
            assert!(options.validate().is_err());
        }
        options.host = "build.example.com".into();
        assert!(options.validate().is_ok());
        options.identity_file = Some("relative/key".into());
        assert!(options.validate().is_err());
    }
    #[test]
    fn reconnect_is_bounded_and_clipboard_directions_are_independent() {
        let mut options = VncOptions::default();
        options.reconnect.enabled = true;
        options.reconnect.max_attempts = 11;
        assert!(options.validate().is_err());
        options.reconnect.max_attempts = 5;
        assert!(options.validate().is_ok());
        assert_eq!(options.reconnect.delay_seconds(0), 1);
        assert_eq!(options.reconnect.delay_seconds(255), 30);
        assert!(ClipboardMode::SendOnly.send());
        assert!(!ClipboardMode::SendOnly.receive());
        assert!(!ClipboardMode::ReceiveOnly.send());
        assert!(ClipboardMode::ReceiveOnly.receive());
        assert!(!ClipboardMode::Disabled.send() && !ClipboardMode::Disabled.receive());
    }
    #[test]
    fn start_command_needs_ssh_and_one_short_line() {
        let mut options = VncOptions {
            ssh_start_command: "~/bin/vnc2039".into(),
            ..VncOptions::default()
        };
        assert!(options.validate().is_err(), "direct transport");
        options.transport = Transport::Ssh;
        options.ssh.host = "build.example.com".into();
        assert!(options.validate().is_ok());
        options.ssh_start_command = "a\nb".into();
        assert!(options.validate().is_err(), "multi-line");
        options.ssh_start_command = "x".repeat(MAX_START_COMMAND + 1);
        assert!(options.validate().is_err(), "too long");
        let json = serde_json::to_string(&VncOptions::default()).unwrap();
        assert!(
            !json.contains("ssh_start_command"),
            "older daemons see no new key"
        );
    }
}
