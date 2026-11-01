//! Connection timing the daemon passes to every OpenSSH call: keep-alive,
//! how long an idle shared connection stays open, and the connect timeout.
//! The daemon sets it from [`Settings`](crate::Settings) at start and on
//! every settings change; until then the defaults apply.
use std::sync::RwLock;

/// `ssh_*` settings and the advertised [`TUNING_CAPABILITY`].
pub const TUNING_CAPABILITY: &str = "ssh-settings-v1";
/// The daemon honors [`Tuning::command_timeout_seconds`].
pub const COMMAND_TIMEOUT_CAPABILITY: &str = "ssh-command-timeout-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tuning {
    /// `ServerAliveInterval`; three missed replies end the connection.
    pub keepalive_seconds: u32,
    /// `ControlPersist` for the shared connection files, Git and new
    /// terminals reuse after its terminal closes.
    pub reuse_minutes: u32,
    /// `ConnectTimeout`.
    pub connect_timeout_seconds: u32,
    /// Limit for one batch call (connection check, login directory, Git
    /// status), sign-in included. Must cover a slow first connection.
    pub command_timeout_seconds: u32,
}

impl Tuning {
    pub const DEFAULT: Self = Self {
        keepalive_seconds: 15,
        reuse_minutes: 10,
        connect_timeout_seconds: 10,
        command_timeout_seconds: 30,
    };
    pub const KEEPALIVE_SECONDS: std::ops::RangeInclusive<u32> = 5..=300;
    pub const REUSE_MINUTES: std::ops::RangeInclusive<u32> = 1..=1440;
    pub const CONNECT_TIMEOUT_SECONDS: std::ops::RangeInclusive<u32> = 3..=120;
    /// Stays below the IPC deadline of SSH project requests.
    pub const COMMAND_TIMEOUT_SECONDS: std::ops::RangeInclusive<u32> = 5..=120;

    pub fn valid(&self) -> bool {
        Self::KEEPALIVE_SECONDS.contains(&self.keepalive_seconds)
            && Self::REUSE_MINUTES.contains(&self.reuse_minutes)
            && Self::CONNECT_TIMEOUT_SECONDS.contains(&self.connect_timeout_seconds)
            && Self::COMMAND_TIMEOUT_SECONDS.contains(&self.command_timeout_seconds)
    }

    #[must_use]
    pub fn command_timeout(self) -> std::time::Duration {
        std::time::Duration::from_secs(u64::from(self.command_timeout_seconds))
    }

    pub(super) fn connect_args(self) -> [String; 3] {
        [
            format!("-oConnectTimeout={}", self.connect_timeout_seconds),
            format!("-oServerAliveInterval={}", self.keepalive_seconds),
            "-oServerAliveCountMax=3".into(),
        ]
    }

    pub(super) fn persist_arg(self) -> String {
        format!("-oControlPersist={}m", self.reuse_minutes)
    }
}

impl Default for Tuning {
    fn default() -> Self {
        Self::DEFAULT
    }
}

static CURRENT: RwLock<Tuning> = RwLock::new(Tuning::DEFAULT);

/// Applies to OpenSSH processes started after this call. Invalid values
/// are ignored.
pub fn set_tuning(tuning: Tuning) {
    if tuning.valid() {
        *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = tuning;
    }
}

/// The tuning that OpenSSH processes started now use.
pub fn current() -> Tuning {
    *CURRENT.read().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tuning_becomes_openssh_options_and_rejects_out_of_range_values() {
        let tuning = Tuning {
            keepalive_seconds: 30,
            reuse_minutes: 60,
            connect_timeout_seconds: 5,
            command_timeout_seconds: 60,
        };
        assert_eq!(
            tuning.connect_args(),
            [
                "-oConnectTimeout=5",
                "-oServerAliveInterval=30",
                "-oServerAliveCountMax=3"
            ]
        );
        assert_eq!(tuning.persist_arg(), "-oControlPersist=60m");
        assert!(Tuning::DEFAULT.valid());
        let settings = crate::Settings {
            ssh_reuse_minutes: 0,
            ..crate::Settings::default()
        };
        assert!(settings.validate().is_err());
        assert_eq!(tuning.command_timeout(), std::time::Duration::from_mins(1));
        let short = Tuning {
            command_timeout_seconds: 1,
            ..Tuning::DEFAULT
        };
        assert!(!short.valid());
    }
}
