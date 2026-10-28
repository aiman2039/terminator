//! Local port forwarding (`ssh -L`) through a project's OpenSSH master.
//! Listeners always bind `127.0.0.1`, never a wildcard address.
use serde::{Deserialize, Serialize};

/// Why a port forward or a project's forward list is invalid.
#[derive(Debug, thiserror::Error)]
pub enum ForwardError {
    #[error("Forwarded ports must be between 1 and 65535")]
    InvalidPort { local_port: u16, remote_port: u16 },
    #[error("Invalid forward destination host")]
    InvalidDestination { host: String },
    #[error("At most {MAX_FORWARDS} port forwards per SSH project")]
    TooMany { count: usize },
    #[error("Local port {local_port} is forwarded twice")]
    DuplicateLocalPort { local_port: u16 },
}

/// `StartForward`, `StopForward`, `ForwardStatus` and
/// `SetRemoteProjectForwards`.
pub const FORWARDING_CAPABILITY: &str = "ssh-forwarding-v1";
pub const MAX_FORWARDS: usize = 32;
/// The only address forward listeners bind.
pub const BIND_ADDRESS: &str = "127.0.0.1";

/// `127.0.0.1:local_port` on this computer reaches `remote_host:remote_port`
/// as seen from the SSH host.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Forward {
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
    /// Started whenever a terminal for the project opens.
    #[serde(default)]
    pub auto_start: bool,
}
impl Forward {
    pub fn validate(&self) -> Result<(), ForwardError> {
        if self.local_port == 0 || self.remote_port == 0 {
            return Err(ForwardError::InvalidPort {
                local_port: self.local_port,
                remote_port: self.remote_port,
            });
        }
        crate::ssh::validate_host(&self.remote_host).map_err(|_| {
            ForwardError::InvalidDestination {
                host: self.remote_host.clone(),
            }
        })?;
        Ok(())
    }
    /// The `-L` operand. The bind address is fixed to [`BIND_ADDRESS`]; an
    /// IPv6 destination is bracketed so its colons stay unambiguous.
    #[must_use]
    pub fn spec(&self) -> String {
        let host = if self.remote_host.contains(':') {
            format!("[{}]", self.remote_host)
        } else {
            self.remote_host.clone()
        };
        format!(
            "{BIND_ADDRESS}:{}:{host}:{}",
            self.local_port, self.remote_port
        )
    }
    /// Whether `other` forwards the same endpoints, ignoring `auto_start`.
    #[must_use]
    pub fn same_endpoints(&self, other: &Self) -> bool {
        self.local_port == other.local_port
            && self.remote_host == other.remote_host
            && self.remote_port == other.remote_port
    }
    #[must_use]
    pub fn label(&self) -> String {
        format!(
            "localhost:{} → {}:{}",
            self.local_port, self.remote_host, self.remote_port
        )
    }
}

/// A project's list: each entry valid, at most [`MAX_FORWARDS`], and no two
/// sharing a local port.
pub fn validate_forwards(forwards: &[Forward]) -> Result<(), ForwardError> {
    if forwards.len() > MAX_FORWARDS {
        return Err(ForwardError::TooMany {
            count: forwards.len(),
        });
    }
    for (index, forward) in forwards.iter().enumerate() {
        forward.validate()?;
        if forwards
            .get(..index)
            .unwrap_or_default()
            .iter()
            .any(|f| f.local_port == forward.local_port)
        {
            return Err(ForwardError::DuplicateLocalPort {
                local_port: forward.local_port,
            });
        }
    }
    Ok(())
}

/// A forward the daemon set up on a live master.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveForward {
    pub project: String,
    pub forward: Forward,
}

/// A forward that failed to start in the background (`auto_start`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardFailure {
    pub project: String,
    pub local_port: u16,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardStatus {
    pub active: Vec<ActiveForward>,
    #[serde(default)]
    pub failures: Vec<ForwardFailure>,
}
impl ForwardStatus {
    #[must_use]
    pub fn is_active(&self, project: &str, local_port: u16) -> bool {
        self.active
            .iter()
            .any(|a| a.project == project && a.forward.local_port == local_port)
    }
    #[must_use]
    pub fn failure(&self, project: &str, local_port: u16) -> Option<&str> {
        self.failures
            .iter()
            .find(|f| f.project == project && f.local_port == local_port)
            .map(|f| f.message.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn forward(local_port: u16, remote_host: &str, remote_port: u16) -> Forward {
        Forward {
            local_port,
            remote_host: remote_host.into(),
            remote_port,
            auto_start: false,
        }
    }

    #[test]
    fn forward_validation_rejects_zero_ports_bad_hosts_and_duplicate_local_ports() {
        assert!(forward(8080, "localhost", 80).validate().is_ok());
        assert!(forward(8080, "::1", 80).validate().is_ok());
        assert!(forward(0, "localhost", 80).validate().is_err());
        assert!(forward(8080, "localhost", 0).validate().is_err());
        for host in ["", "-oProxyCommand=x", "a b", "a/b", "*", "a\nb"] {
            assert!(forward(8080, host, 80).validate().is_err(), "{host:?}");
        }
        assert!(validate_forwards(&[forward(8080, "a", 80), forward(8081, "a", 80)]).is_ok());
        assert!(validate_forwards(&[forward(8080, "a", 80), forward(8080, "b", 81)]).is_err());
        let many: Vec<_> = (1..=u16::try_from(MAX_FORWARDS + 1).unwrap())
            .map(|p| forward(p, "a", 80))
            .collect();
        assert!(validate_forwards(&many).is_err());
    }

    #[test]
    fn forward_spec_binds_loopback_only() {
        assert_eq!(
            forward(8080, "db.internal", 5432).spec(),
            "127.0.0.1:8080:db.internal:5432"
        );
        assert_eq!(forward(9000, "::1", 80).spec(), "127.0.0.1:9000:[::1]:80");
        for spec in [
            forward(1, "h", 2).spec(),
            forward(65535, "::", 65535).spec(),
        ] {
            assert!(spec.starts_with("127.0.0.1:"), "{spec}");
            assert!(!spec.starts_with('*') && !spec.starts_with("0.0.0.0"));
        }
    }

    #[test]
    fn auto_start_defaults_off_and_status_lookups_match_project_and_port() {
        let f: Forward =
            serde_json::from_str(r#"{"local_port":1,"remote_host":"h","remote_port":2}"#).unwrap();
        assert!(!f.auto_start);
        let status = ForwardStatus {
            active: vec![ActiveForward {
                project: "a".into(),
                forward: f.clone(),
            }],
            failures: vec![ForwardFailure {
                project: "b".into(),
                local_port: 1,
                message: "in use".into(),
            }],
        };
        assert!(status.is_active("a", 1));
        assert!(!status.is_active("b", 1));
        assert_eq!(status.failure("b", 1), Some("in use"));
        assert_eq!(status.failure("a", 1), None);
        let legacy: ForwardStatus = serde_json::from_str(r#"{"active":[]}"#).unwrap();
        assert!(legacy.failures.is_empty());
    }
}
