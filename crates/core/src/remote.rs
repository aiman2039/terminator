//! Remote roots never become local project filesystem paths.
use crate::ssh::SshOptions;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub const CAPABILITY: &str = "ssh-projects-v1";
pub const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 1000;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteProject {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub ssh: SshOptions,
    pub root: String,
    #[serde(default)]
    pub layout: serde_json::Value,
}
impl RemoteProject {
    pub fn validate(&self) -> Result<()> {
        self.ssh.validate()?;
        ensure!(
            self.id.len() <= 128
                && self
                    .id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c)),
            "Invalid remote project ID"
        );
        ensure!(
            !self.name.trim().is_empty()
                && self.name.len() <= 255
                && !self.name.chars().any(char::is_control),
            "Remote project name must be 1–255 characters without controls"
        );
        ensure!(
            self.root.starts_with('/')
                && self.root.len() <= 4096
                && !self.root.contains('\\')
                && !self.root.chars().any(char::is_control)
                && !self.root.split('/').any(|s| s == ".."),
            "Remote root must be an absolute POSIX path without parent traversal"
        );
        Ok(())
    }
    pub fn absolute(&self, relative: &str) -> Result<String> {
        validate_relative(relative)?;
        Ok(if relative.is_empty() {
            self.root.trim_end_matches('/').to_owned() + "/"
        } else {
            format!("{}/{}", self.root.trim_end_matches('/'), relative)
        })
    }
    pub fn label(&self) -> String {
        format!(
            "ssh://{}@{}:{}{}",
            self.ssh.username, self.ssh.host, self.ssh.port, self.root
        )
    }
    /// A non-filesystem identity for older GUI model code. Never used as cwd.
    pub fn identity_path(&self) -> std::path::PathBuf {
        std::path::PathBuf::from(format!("/__terminator_remote__/{}", self.id))
    }
}
pub fn validate_relative(path: &str) -> Result<()> {
    ensure!(
        path.len() <= 4096
            && !path.starts_with('/')
            && !path.contains('\\')
            && !path.chars().any(char::is_control)
            && !path.split('/').any(|part| part == ".." || part == "."),
        "Remote path must be relative and contain no parent traversal or controls"
    );
    Ok(())
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
    List { path: String },
    Read { path: String },
    GitStatus,
    GitDiff { path: String, staged: bool },
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
}

#[cfg(test)]
mod tests {
    use super::*;
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
        };
        assert_ne!(p.identity_path(), std::path::PathBuf::from(&p.root));
        assert_eq!(
            p.absolute("src/main.rs").unwrap(),
            "/workspace/project/src/main.rs"
        );
        assert!(p.absolute("../local").is_err());
    }
}
