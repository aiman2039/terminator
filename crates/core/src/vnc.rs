use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub const CAPABILITY: &str = "vnc-v1";
pub const DEFAULT_PORT: u16 = 5900;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VncProfile {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default = "default_true")]
    pub shared: bool,
    #[serde(default)]
    pub view_only: bool,
    #[serde(default)]
    pub last_connected: u64,
}

fn default_true() -> bool {
    true
}

impl VncProfile {
    pub fn validate(&self) -> Result<()> {
        ensure!(!self.id.trim().is_empty(), "VNC profile ID is required");
        ensure!(
            !self.host.trim().is_empty() && self.host.len() <= 255,
            "VNC host must be 1–255 characters"
        );
        ensure!(self.port > 0, "VNC port must be between 1 and 65535");
        ensure!(
            self.name.len() <= 255,
            "VNC profile name must be at most 255 characters"
        );
        Ok(())
    }

    #[must_use]
    pub fn display_name(&self) -> String {
        let trimmed = self.name.trim();
        if trimmed.is_empty() {
            format!("{}:{}", self.host.trim(), self.port)
        } else {
            trimmed.to_string()
        }
    }

    #[must_use]
    pub fn endpoint(&self) -> String {
        let host = self.host.trim();
        if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]:{}", self.port)
        } else {
            format!("{host}:{}", self.port)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VncSessionStatus {
    Connecting,
    Connected,
    Disconnected,
    Failed,
}

impl VncSessionStatus {
    #[must_use]
    pub fn live(self) -> bool {
        matches!(self, Self::Connecting | Self::Connected)
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Connecting => "Connecting",
            Self::Connected => "Connected",
            Self::Disconnected => "Disconnected",
            Self::Failed => "Failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VncSession {
    pub id: String,
    pub project_id: String,
    #[serde(default)]
    pub profile_id: Option<String>,
    pub label: String,
    pub host: String,
    pub port: u16,
    pub width: u16,
    pub height: u16,
    #[serde(default)]
    pub view_only: bool,
    pub status: VncSessionStatus,
    #[serde(default)]
    pub error: Option<String>,
    pub created: u64,
    pub generation: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VncInputEvent {
    Pointer { x: u16, y: u16, buttons: u8 },
    Key { keycode: u32, down: bool },
    CopyText(String),
    Refresh { full: bool },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VncFrame {
    pub session: String,
    pub width: u16,
    pub height: u16,
    pub revision: u64,
    pub status: VncSessionStatus,
    #[serde(default)]
    pub error: Option<String>,
    /// Zlib-compressed RGBA pixels, base64-encoded. Empty when width or height is 0.
    #[serde(default)]
    pub zlib_rgba: String,
    #[serde(default)]
    pub clipboard: Option<String>,
}

/// Parse a `host` or `host:port` string into `(host, port)`.
#[must_use]
pub fn parse_host_port(input: &str, fallback_port: u16) -> (String, u16) {
    let trimmed = input.trim();
    let trimmed = trimmed
        .strip_prefix("vnc://")
        .or_else(|| trimmed.strip_prefix("rfb://"))
        .unwrap_or(trimmed);
    if let Some(rest) = trimmed.strip_prefix('[')
        && let Some((host, tail)) = rest.split_once(']')
    {
        let port = tail
            .strip_prefix(':')
            .and_then(|p| p.trim().parse::<u16>().ok())
            .filter(|p| *p > 0)
            .unwrap_or(fallback_port);
        return (host.to_string(), port);
    }
    if let Some((host, port_str)) = trimmed.rsplit_once(':')
        && !host.contains(':')
        && let Ok(port) = port_str.trim().parse::<u16>()
        && port > 0
    {
        return (host.trim().to_string(), port);
    }
    (
        trimmed.to_string(),
        if fallback_port == 0 {
            DEFAULT_PORT
        } else {
            fallback_port
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_host_port_handles_bare_hosts_ports_and_ipv6() {
        assert_eq!(
            parse_host_port("127.0.0.1", 5900),
            ("127.0.0.1".into(), 5900)
        );
        assert_eq!(
            parse_host_port("vnc://10.0.0.5:5901", 5900),
            ("10.0.0.5".into(), 5901)
        );
        assert_eq!(parse_host_port("[::1]:5902", 5900), ("::1".into(), 5902));
        assert_eq!(parse_host_port("::1", 5900), ("::1".into(), 5900));
    }

    #[test]
    fn profile_validation_rejects_empty_host_or_zero_port() {
        let mut profile = VncProfile {
            id: "p1".into(),
            name: "Dev VM".into(),
            host: "127.0.0.1".into(),
            port: 5900,
            username: String::new(),
            password: "secret".into(),
            shared: true,
            view_only: false,
            last_connected: 0,
        };
        assert!(profile.validate().is_ok());
        assert_eq!(profile.display_name(), "Dev VM");
        assert_eq!(profile.endpoint(), "127.0.0.1:5900");

        profile.host.clear();
        assert!(profile.validate().is_err());
        profile.host = "127.0.0.1".into();
        profile.port = 0;
        assert!(profile.validate().is_err());
    }
}
