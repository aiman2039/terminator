//! Remote roots never become local project filesystem paths.
use crate::ssh::SshOptions;
use serde::{Deserialize, Serialize};

mod commands;
mod forward;
pub mod mosh;
mod paths;
pub use commands::{
    COMMANDS_CAPABILITY, DEFAULT_SHELL_COMMAND, MAX_COMMAND_LEN, normalize_command,
};
pub use forward::{
    ActiveForward, BIND_ADDRESS, FORWARDING_CAPABILITY, Forward, ForwardError, ForwardFailure,
    ForwardStatus, MAX_FORWARDS, validate_forwards,
};
pub use paths::{validate_file_name, validate_relative};

pub const CAPABILITY: &str = "ssh-projects-v1";
/// `RemoveRemoteProject`, `SetRemoteProjectGroup` and [`HOME_ROOT`] probes.
pub const HOSTS_CAPABILITY: &str = "ssh-hosts-v1";
/// `UpdateRemoteProject` and [`RemoteProject::last_connected`].
pub const EDIT_CAPABILITY: &str = "ssh-edit-v1";
/// [`RemoteProject::command_timeout_seconds`] is stored and honored.
pub const HOST_TIMEOUT_CAPABILITY: &str = "ssh-host-timeout-v1";
/// [`RemoteProject::mosh`] is stored and SSH terminals honor it.
pub const MOSH_CAPABILITY: &str = "ssh-mosh-v1";
/// [`Operation::Upload`], [`Operation::Download`] and
/// [`Operation::TransferStatus`].
pub const TRANSFER_CAPABILITY: &str = "ssh-transfer-v1";
/// A `ProbeRemoteProject` root that resolves the login directory instead of
/// a path. Never stored: the probe replies with the absolute path.
pub const HOME_ROOT: &str = "~";
pub const MAX_GROUP_LEN: usize = 64;
pub const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 1000;

/// Why a remote project, group, or path is invalid.
#[derive(Debug, thiserror::Error)]
pub enum RemoteError {
    #[error(transparent)]
    Ssh(#[from] crate::ssh::SshOptionsError),
    #[error(transparent)]
    Forward(#[from] ForwardError),
    #[error("Group names must be 1–64 characters without controls")]
    InvalidGroup { group: String },
    #[error("Group names must be trimmed and non-empty")]
    UntrimmedGroup { group: Option<String> },
    #[error("Invalid remote project ID")]
    InvalidId { id: String },
    #[error("Remote project name must be 1–255 characters without controls")]
    InvalidName { name: String },
    #[error("Remote root must be an absolute POSIX path without parent traversal")]
    InvalidRoot { root: String },
    #[error("Remote path must be relative and contain no parent traversal or controls")]
    InvalidRelativePath { path: String },
    #[error("File name must be one path component without controls")]
    InvalidFileName { name: String },
    #[error("SSH command timeout must be between 5 and 120 seconds")]
    InvalidTimeout { seconds: u32 },
    #[error(
        "Host commands must be one trimmed, non-empty line of at most {} bytes",
        commands::MAX_COMMAND_LEN
    )]
    InvalidCommand { command: String },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteProject {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub ssh: SshOptions,
    pub root: String,
    #[serde(default)]
    pub layout: serde_json::Value,
    /// Hosts sidebar folder. Omitted when unset so older readers see the
    /// same JSON they always did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Local port forwards (`ssh-forwarding-v1`). Omitted when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forwards: Vec<Forward>,
    /// Unix seconds when an SSH terminal last opened (`ssh-edit-v1`); 0 is
    /// never. Set by the daemon only.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub last_connected: u64,
    /// Limit for one batch SSH call to this host; `None` uses Settings →
    /// SSH (`ssh-host-timeout-v1`). Omitted when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_timeout_seconds: Option<u32>,
    /// New terminals and editors run through Mosh (`ssh-mosh-v1`): SSH
    /// starts `mosh-server` on the host, then a local `mosh-client` carries
    /// the session over UDP. Explorer, Git and transfers stay on SSH.
    /// Omitted when off.
    #[serde(default, skip_serializing_if = "is_false")]
    pub mosh: bool,
    /// Host command a new SSH or Mosh shell terminal runs after `cd` to its
    /// folder (`ssh-commands-v1`), for example `tmux new -A -s main`.
    /// `None` starts the login shell ([`DEFAULT_SHELL_COMMAND`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell_command: Option<String>,
    /// `mosh-server` command line (`ssh-commands-v1`); the daemon adds
    /// `-- sh -c <script>`. `None` uses [`mosh::default_server_command`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mosh_command: Option<String>,
    /// Host command that starts the remote-apps bridge (`remote-apps-v1`),
    /// for example through nix-portable. `None` uses
    /// [`crate::remote_apps::DEFAULT_COMMAND`]. Omitted when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apps_command: Option<String>,
}
fn is_zero(value: &u64) -> bool {
    *value == 0
}
fn is_false(value: &bool) -> bool {
    !*value
}
/// Trims a group name; blank means ungrouped.
pub fn normalize_group(group: Option<&str>) -> Result<Option<String>, RemoteError> {
    let Some(group) = group.map(str::trim).filter(|g| !g.is_empty()) else {
        return Ok(None);
    };
    if group.len() > MAX_GROUP_LEN || group.chars().any(char::is_control) {
        return Err(RemoteError::InvalidGroup {
            group: group.to_owned(),
        });
    }
    Ok(Some(group.to_owned()))
}
impl RemoteProject {
    pub fn validate(&self) -> Result<(), RemoteError> {
        self.ssh.validate()?;
        if normalize_group(self.group.as_deref())? != self.group {
            return Err(RemoteError::UntrimmedGroup {
                group: self.group.clone(),
            });
        }
        validate_forwards(&self.forwards)?;
        if let Some(seconds) = self.command_timeout_seconds
            && !crate::ssh::Tuning::COMMAND_TIMEOUT_SECONDS.contains(&seconds)
        {
            return Err(RemoteError::InvalidTimeout { seconds });
        }
        commands::validate_commands(self)?;
        let valid_id = self.id.len() <= 128
            && self
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c));
        if !valid_id {
            return Err(RemoteError::InvalidId {
                id: self.id.clone(),
            });
        }
        let valid_name = !self.name.trim().is_empty()
            && self.name.len() <= 255
            && !self.name.chars().any(char::is_control);
        if !valid_name {
            return Err(RemoteError::InvalidName {
                name: self.name.clone(),
            });
        }
        let valid_root = self.root.starts_with('/')
            && self.root.len() <= 4096
            && !self.root.contains('\\')
            && !self.root.chars().any(char::is_control)
            && !self.root.split('/').any(|s| s == "..");
        if !valid_root {
            return Err(RemoteError::InvalidRoot {
                root: self.root.clone(),
            });
        }
        Ok(())
    }
    /// This host's batch-call limit, else the Settings → SSH one.
    #[must_use]
    pub fn command_timeout(&self) -> std::time::Duration {
        self.command_timeout_seconds.map_or_else(
            || crate::ssh::current_tuning().command_timeout(),
            |seconds| std::time::Duration::from_secs(u64::from(seconds)),
        )
    }
    pub fn absolute(&self, relative: &str) -> Result<String, RemoteError> {
        validate_relative(relative)?;
        Ok(if relative.is_empty() {
            self.root.trim_end_matches('/').to_owned() + "/"
        } else {
            format!("{}/{}", self.root.trim_end_matches('/'), relative)
        })
    }
    /// Whether `other` connects the same way: host, user, port, keys and
    /// root. Name, group, forwards and times do not count.
    pub fn same_connection(&self, other: &Self) -> bool {
        self.ssh == other.ssh && self.root == other.root
    }
    pub fn label(&self) -> String {
        format!("ssh://{}{}", self.ssh.authority(), self.root)
    }
    /// A non-filesystem identity for older GUI model code. Never used as cwd.
    pub fn identity_path(&self) -> std::path::PathBuf {
        std::path::PathBuf::from(format!("/__terminator_remote__/{}", self.id))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteSession {
    pub ssh: SshOptions,
    pub root: String,
    pub cwd: String,
    pub editor_socket: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRef {
    pub project: String,
    pub path: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operation {
    List {
        path: String,
    },
    Read {
        path: String,
    },
    GitStatus,
    GitDiff {
        path: String,
        staged: bool,
    },
    /// Starts a background SFTP upload of the local file `local` into the
    /// project directory `remote_dir` ("" is the root), keeping its file
    /// name. Replies [`Reply::Transfer`] at once; poll
    /// [`Operation::TransferStatus`]. An existing remote file is never
    /// replaced. Requires [`TRANSFER_CAPABILITY`].
    Upload {
        local: std::path::PathBuf,
        remote_dir: String,
    },
    /// Starts a background SFTP download of the project file `path` to the
    /// new absolute local file `local`. An existing local file is never
    /// replaced. Replies [`Reply::Transfer`]. Requires
    /// [`TRANSFER_CAPABILITY`].
    Download {
        path: String,
        local: std::path::PathBuf,
    },
    /// Replies [`Reply::Transfer`] for a transfer this daemon started.
    TransferStatus {
        id: String,
    },
}
impl Operation {
    /// Upload and Download start work; every other operation only reads.
    #[must_use]
    pub fn starts_transfer(&self) -> bool {
        matches!(self, Self::Upload { .. } | Self::Download { .. })
    }
}
/// Progress of a project file transfer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferStatus {
    pub id: String,
    pub upload: bool,
    /// The transferred file's name, for display.
    pub name: String,
    /// Project-relative directory an upload targets; empty for downloads.
    #[serde(default)]
    pub remote_dir: String,
    pub done: bool,
    pub bytes: u64,
    /// Source size, once known.
    #[serde(default)]
    pub total: Option<u64>,
    pub error: Option<String>,
    /// Set when `error` is a classified SSH connection failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh: Option<crate::ssh::SshError>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub directory: bool,
    pub symlink: bool,
    pub bytes: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub path: String,
    pub status: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Probe {
        root: String,
    },
    Directory {
        path: String,
        entries: Vec<Entry>,
        /// More than the listing limit exist; only the first ones are sent.
        /// Older daemons omit it.
        #[serde(default)]
        truncated: bool,
        /// Entries left out because their name is not UTF-8 or not a single
        /// path component. Older daemons omit it.
        #[serde(default)]
        skipped: usize,
    },
    File {
        path: String,
        base64: String,
    },
    Git {
        branch: String,
        changes: Vec<Change>,
    },
    Diff {
        path: String,
        left: String,
        right: String,
        staged: bool,
    },
    Transfer(TransferStatus),
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn directory_reply_from_older_daemon_is_complete() {
        let old = r#"{"Directory":{"path":"src","entries":[]}}"#;
        let reply: Reply = serde_json::from_str(old).unwrap();
        assert_eq!(
            reply,
            Reply::Directory {
                path: "src".into(),
                entries: Vec::new(),
                truncated: false,
                skipped: 0,
            }
        );
    }
    #[test]
    fn remote_paths_cannot_escape_or_become_local_roots() {
        for path in [
            "../file",
            "/tmp/local",
            "a/../../file",
            "a\\file",
            "a\nfile",
        ] {
            assert!(validate_relative(path).is_err(), "{path:?}");
        }
        assert!(validate_relative("path with spaces/café.txt").is_ok());
        let p = RemoteProject {
            id: "fixture".into(),
            name: "Fixture".into(),
            ssh: SshOptions {
                host: "127.0.0.1".into(),
                ..SshOptions::default()
            },
            root: "/workspace/project".into(),
            layout: serde_json::Value::Null,
            group: None,
            forwards: Vec::new(),
            last_connected: 0,
            command_timeout_seconds: None,
            mosh: false,
            shell_command: None,
            mosh_command: None,
            apps_command: None,
        };
        assert_ne!(p.identity_path(), std::path::PathBuf::from(&p.root));
        assert_eq!(
            p.absolute("src/main.rs").unwrap(),
            "/workspace/project/src/main.rs"
        );
        assert!(p.absolute("../local").is_err());
    }
    #[test]
    fn host_timeout_is_optional_bounded_and_overrides_settings() {
        let mut p = RemoteProject {
            id: "fixture".into(),
            name: "Fixture".into(),
            ssh: SshOptions {
                host: "slow.example.com".into(),
                ..SshOptions::default()
            },
            root: "/home/me".into(),
            layout: serde_json::Value::Null,
            group: None,
            forwards: Vec::new(),
            last_connected: 0,
            command_timeout_seconds: None,
            mosh: false,
            shell_command: None,
            mosh_command: None,
            apps_command: None,
        };
        // Unset: older readers see the same JSON, and Settings decides.
        assert!(
            !serde_json::to_string(&p)
                .unwrap()
                .contains("command_timeout")
        );
        assert_eq!(
            p.command_timeout(),
            crate::ssh::current_tuning().command_timeout()
        );
        p.command_timeout_seconds = Some(90);
        assert!(p.validate().is_ok());
        assert_eq!(p.command_timeout(), std::time::Duration::from_secs(90));
        let json = serde_json::to_string(&p).unwrap();
        let back: RemoteProject = serde_json::from_str(&json).unwrap();
        assert_eq!(back.command_timeout_seconds, Some(90));
        for bad in [0, 4, 121] {
            p.command_timeout_seconds = Some(bad);
            assert!(p.validate().is_err(), "{bad}");
        }
    }
    #[test]
    fn only_upload_and_download_start_transfers() {
        assert!(
            Operation::Upload {
                local: "/tmp/a".into(),
                remote_dir: String::new()
            }
            .starts_transfer()
        );
        assert!(
            Operation::Download {
                path: "a".into(),
                local: "/tmp/a".into()
            }
            .starts_transfer()
        );
        assert!(!Operation::TransferStatus { id: "x".into() }.starts_transfer());
        assert!(
            !Operation::List {
                path: String::new()
            }
            .starts_transfer()
        );
    }
    #[test]
    fn group_defaults_to_none_and_is_omitted_when_unset() {
        let legacy = r#"{"id":"a","name":"A","ssh":{"host":"h"},"root":"/r"}"#;
        let mut p: RemoteProject = serde_json::from_str(legacy).unwrap();
        assert_eq!(p.group, None);
        assert!(p.validate().is_ok());
        assert!(!serde_json::to_string(&p).unwrap().contains("group"));
        p.group = Some("Prod".into());
        let back: RemoteProject =
            serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back.group.as_deref(), Some("Prod"));
        p.group = Some(" Prod ".into());
        assert!(p.validate().is_err());
        assert_eq!(normalize_group(Some("  ")).unwrap(), None);
        assert_eq!(
            normalize_group(Some(" Prod ")).unwrap().as_deref(),
            Some("Prod")
        );
        assert!(normalize_group(Some("a\nb")).is_err());
    }
    #[test]
    fn forwards_default_empty_are_omitted_and_validated_with_the_project() {
        let legacy = r#"{"id":"a","name":"A","ssh":{"host":"h"},"root":"/r"}"#;
        let mut p: RemoteProject = serde_json::from_str(legacy).unwrap();
        assert!(p.forwards.is_empty());
        assert!(!serde_json::to_string(&p).unwrap().contains("forwards"));
        let forward = Forward {
            local_port: 8080,
            remote_host: "localhost".into(),
            remote_port: 80,
            auto_start: true,
        };
        p.forwards = vec![forward.clone()];
        let back: RemoteProject =
            serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back.forwards, vec![forward.clone()]);
        p.forwards.push(forward);
        assert!(p.validate().is_err());
    }
}
