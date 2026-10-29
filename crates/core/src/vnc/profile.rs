//! Saved VNC hosts ("profiles") and their validation.
use super::{VncOptions, VncOptionsError};
use serde::{Deserialize, Serialize};

/// Saved hosts keep `labels`; older daemons drop the field when they save.
pub const LABELS_CAPABILITY: &str = "vnc-labels-v1";
pub const MAX_LABELS: usize = 16;
pub const MAX_LABEL_LEN: usize = 64;

/// Why a saved VNC profile is invalid.
#[derive(Debug, thiserror::Error)]
pub enum VncProfileError {
    #[error("Invalid VNC profile ID")]
    InvalidId { id: String },
    #[error("Invalid VNC username")]
    InvalidUsername { username: String },
    #[error("VNC password exceeds 4096 bytes")]
    PasswordTooLong { bytes: usize },
    #[error("VNC host must be 1–255 characters")]
    InvalidHost { host: String },
    #[error("VNC port must be between 1 and 65535")]
    InvalidPort,
    #[error("VNC profile name must be at most 255 characters")]
    NameTooLong { bytes: usize },
    #[error("A VNC host can have at most 16 labels of 1–64 characters")]
    InvalidLabels,
    #[error(transparent)]
    Options(#[from] VncOptionsError),
}

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
    #[serde(default)]
    pub options: VncOptions,
    #[serde(default)]
    pub credential_saved: bool,
    /// Address-book groups, for example `lab` or `team/build`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
}

fn default_true() -> bool {
    true
}

impl VncProfile {
    pub fn validate(&self) -> Result<(), VncProfileError> {
        let valid_id = !self.id.is_empty()
            && self.id.len() <= 128
            && self
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
        if !valid_id {
            return Err(VncProfileError::InvalidId {
                id: self.id.clone(),
            });
        }
        if self.username.len() > 4096 || self.username.chars().any(char::is_control) {
            return Err(VncProfileError::InvalidUsername {
                username: self.username.clone(),
            });
        }
        if self.password.len() > 4096 {
            return Err(VncProfileError::PasswordTooLong {
                bytes: self.password.len(),
            });
        }
        if self.host.trim().is_empty() || self.host.len() > 255 {
            return Err(VncProfileError::InvalidHost {
                host: self.host.clone(),
            });
        }
        if self.port == 0 {
            return Err(VncProfileError::InvalidPort);
        }
        if self.name.len() > 255 {
            return Err(VncProfileError::NameTooLong {
                bytes: self.name.len(),
            });
        }
        let valid_labels = self.labels.len() <= MAX_LABELS
            && self.labels.iter().all(|label| {
                !label.trim().is_empty()
                    && label.len() <= MAX_LABEL_LEN
                    && !label.chars().any(char::is_control)
            });
        if !valid_labels {
            return Err(VncProfileError::InvalidLabels);
        }
        self.options.validate()?;
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
