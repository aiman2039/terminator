mod errors;
mod options;
mod profile;
pub use errors::*;
pub use options::*;
pub use profile::*;

use serde::{Deserialize, Serialize};

pub const CAPABILITY: &str = "vnc-v1";
/// Incremental frame updates: the daemon may answer a frame request that
/// carries `regions: true` with only the changed rectangle.
pub const REGIONS_CAPABILITY: &str = "vnc-regions-v1";
pub const DEFAULT_PORT: u16 = 5900;
/// Shared-memory frames, long-polled frame requests, and cursor images sent
/// only when they change. See [`VncShmRequest`] and [`VncShmFrame`].
pub const SHM_CAPABILITY: &str = "vnc-shm-v1";
/// Longest a shared-memory frame request waits for a new revision.
pub const MAX_FRAME_WAIT_MS: u32 = 500;
/// Most changed rectangles one shared-memory frame lists.
pub const MAX_SHM_REGIONS: usize = 32;

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
    #[serde(default)]
    pub options: VncOptions,
    #[serde(default)]
    pub retry_attempt: u8,
    #[serde(default)]
    pub retry_in_seconds: u64,
    /// Why the session failed (`vnc-errors-v1` daemons); omitted otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<VncErrorKind>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VncInputEvent {
    Pointer { x: u16, y: u16, buttons: u8 },
    Key { keycode: u32, down: bool },
    CopyText(String),
    Refresh { full: bool },
    Resize { width: u16, height: u16 },
    TypeText(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VncRegion {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl VncRegion {
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Smallest rectangle covering both inputs.
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = self
            .x
            .saturating_add(self.width)
            .max(other.x.saturating_add(other.width));
        let bottom = self
            .y
            .saturating_add(self.height)
            .max(other.y.saturating_add(other.height));
        Self {
            x,
            y,
            width: right.saturating_sub(x),
            height: bottom.saturating_sub(y),
        }
    }
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
    /// With `region` set, the pixels cover that rectangle instead of the
    /// whole frame; `width`/`height` always describe the full frame.
    #[serde(default)]
    pub zlib_rgba: String,
    /// Changed rectangle inside the frame, for incremental updates
    /// (`regions: true` on the request). Absent means the pixels cover
    /// the whole frame.
    #[serde(default)]
    pub region: Option<VncRegion>,
    #[serde(default)]
    pub clipboard: Option<String>,
    #[serde(default)]
    pub cursor: Option<VncCursor>,
    #[serde(default)]
    pub monitors: Vec<VncMonitor>,
    #[serde(default)]
    pub resize_supported: bool,
    /// Pixels are in a shared-memory file instead of `zlib_rgba`. Only sent
    /// to a request that carried [`VncShmRequest`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shm: Option<VncShmFrame>,
    /// Counter of cursor image changes. With [`VncShmRequest`], `cursor` is
    /// sent only when this is newer than the request's `cursor_serial`.
    #[serde(default)]
    pub cursor_serial: u64,
}

/// Shared-memory options of a frame request (`vnc-shm-v1` daemons only).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VncShmRequest {
    /// Wait up to this long (capped at [`MAX_FRAME_WAIT_MS`]) for a revision
    /// newer than `since` before answering `Unchanged`.
    #[serde(default)]
    pub wait_ms: u32,
    /// The newest cursor the viewer has; older or equal cursors are omitted.
    #[serde(default)]
    pub cursor_serial: u64,
}

/// Where a shared-memory frame's pixels are: the file holds the whole
/// framebuffer after the daemon copied `regions` (the whole frame when
/// empty) under seqlock `sequence`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VncShmFrame {
    pub path: std::path::PathBuf,
    pub sequence: u64,
    #[serde(default)]
    pub regions: Vec<VncRegion>,
    /// Server-side speed, for the viewer's speed indicator.
    #[serde(default)]
    pub stats: VncStats,
}

/// How fast the VNC server answers the daemon's update requests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VncStats {
    /// Complete framebuffer updates received in the last second.
    pub updates_per_second: u16,
    /// Smoothed time from an update request to the end of its answer.
    pub update_ms: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VncCursor {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub rgba: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VncMonitor {
    pub id: u32,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferDirection {
    Upload,
    Download,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VncTransfer {
    pub id: String,
    pub direction: TransferDirection,
    pub local: std::path::PathBuf,
    pub remote: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VncTransferStatus {
    pub id: String,
    pub done: bool,
    pub bytes: u64,
    pub error: Option<String>,
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
            options: VncOptions::default(),
            credential_saved: false,
            labels: Vec::new(),
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

    #[test]
    fn profile_labels_are_bounded() {
        let mut profile = VncProfile {
            id: "p".into(),
            name: String::new(),
            host: "h".into(),
            port: 5900,
            username: String::new(),
            password: String::new(),
            shared: true,
            view_only: false,
            last_connected: 0,
            options: VncOptions::default(),
            credential_saved: false,
            labels: vec!["lab".into()],
        };
        assert!(profile.validate().is_ok());
        profile.labels = vec![" ".into()];
        assert!(profile.validate().is_err());
        profile.labels = vec!["x".repeat(MAX_LABEL_LEN + 1)];
        assert!(profile.validate().is_err());
        profile.labels = (0..=MAX_LABELS).map(|n| n.to_string()).collect();
        assert!(profile.validate().is_err());
    }

    #[test]
    fn region_union_covers_both_rectangles() {
        let a = VncRegion {
            x: 4,
            y: 8,
            width: 16,
            height: 16,
        };
        let b = VncRegion {
            x: 0,
            y: 20,
            width: 8,
            height: 4,
        };
        let union = a.union(b);
        assert_eq!(
            union,
            VncRegion {
                x: 0,
                y: 8,
                width: 20,
                height: 16
            }
        );
        assert_eq!(a.union(VncRegion::default()), a);
        assert_eq!(VncRegion::default().union(b), b);
    }

    #[test]
    fn a_frame_without_a_region_still_decodes() {
        // Older daemons never send the field: decode must tolerate both.
        let json = r#"{"session":"s","width":4,"height":4,"revision":1,
            "status":"connected","zlib_rgba":""}"#;
        let frame: VncFrame = serde_json::from_str(json).expect("legacy frame decodes");
        assert_eq!(frame.region, None);
        let with_region = VncFrame {
            region: Some(VncRegion {
                x: 1,
                y: 2,
                width: 3,
                height: 4,
            }),
            ..frame
        };
        let encoded = serde_json::to_string(&with_region).expect("frame encodes");
        let decoded: VncFrame = serde_json::from_str(&encoded).expect("frame decodes");
        assert_eq!(decoded.region, with_region.region);
        assert!(!encoded.contains("shm"), "legacy frames stay unchanged");
    }
}
