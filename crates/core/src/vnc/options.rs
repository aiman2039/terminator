use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const FEATURES_CAPABILITY: &str = "vnc-features-v1";
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
}
impl VncOptions {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.reconnect.max_attempts <= 10,
            "Reconnect attempts must not exceed 10"
        );
        ensure!(
            !self.reconnect.enabled || self.reconnect.max_attempts > 0,
            "Reconnect requires at least one attempt"
        );
        ensure!(
            self.tls_server_name.len() <= 255
                && !self.tls_server_name.chars().any(char::is_control),
            "Invalid TLS server name"
        );
        if let Some(path) = &self.tls_ca_file {
            ensure!(path.is_absolute(), "TLS CA file must be an absolute path");
        }
        if self.transport == Transport::Ssh {
            self.ssh.validate()?;
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
}
