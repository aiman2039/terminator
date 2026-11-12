//! A plain OpenSSH terminal that ended because the connection failed or
//! dropped. The daemon keeps its record so the dead pane can offer an
//! explicit Reconnect; nothing reconnects by itself.
use super::RemoteProject;
use crate::{Lifecycle, Session, SessionKind};

/// The exit status OpenSSH uses for its own errors, a lost connection
/// included (`ssh(1)`: "exits with 255 if an error occurred").
pub const SSH_ERROR_EXIT: u32 = 255;

/// True for an ended OpenSSH shell terminal whose `ssh` reported its own
/// error. Mosh terminals (the host's [`RemoteProject::mosh`]) and editors
/// never count. Editing a host is refused while it has live sessions, so
/// the host's Mosh setting is the one the terminal started with.
#[must_use]
pub fn connection_lost(session: &Session, projects: &[RemoteProject]) -> bool {
    session.remote.is_some()
        && session.kind == SessionKind::Shell
        && session.lifecycle == Lifecycle::Ended
        && session.exit_code == Some(SSH_ERROR_EXIT)
        && projects
            .iter()
            .any(|project| project.id == session.project_id && !project.mosh)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(mosh: bool) -> RemoteProject {
        RemoteProject {
            id: "host".into(),
            name: "Host".into(),
            ssh: crate::ssh::SshOptions {
                host: "fixture".into(),
                ..crate::ssh::SshOptions::default()
            },
            root: "/srv".into(),
            layout: serde_json::Value::Null,
            group: None,
            forwards: Vec::new(),
            last_connected: 0,
            command_timeout_seconds: None,
            mosh,
            shell_command: None,
            mosh_command: None,
            apps_command: None,
        }
    }

    fn ended(exit: Option<u32>, kind: SessionKind) -> Session {
        Session {
            review: false,
            remote: Some(super::super::RemoteSession {
                ssh: host(false).ssh,
                root: "/srv".into(),
                cwd: "/srv".into(),
                editor_socket: None,
            }),
            id: "s".into(),
            project_id: "host".into(),
            label: "s".into(),
            cwd: "/srv".into(),
            kind,
            file: None,
            lifecycle: Lifecycle::Ended,
            created: 0,
            exit_code: exit,
            rows: 24,
            cols: 80,
            generation: "g".into(),
            pid: None,
            truncated: false,
            cwd_confirmed: false,
        }
    }

    #[test]
    fn only_an_ssh_error_exit_of_an_openssh_shell_is_a_lost_connection() {
        let plain = [host(false)];
        assert!(connection_lost(
            &ended(Some(255), SessionKind::Shell),
            &plain
        ));
        assert!(!connection_lost(
            &ended(Some(0), SessionKind::Shell),
            &plain
        ));
        assert!(!connection_lost(&ended(None, SessionKind::Shell), &plain));
        assert!(!connection_lost(
            &ended(Some(255), SessionKind::Editor),
            &plain
        ));
        assert!(!connection_lost(
            &ended(Some(255), SessionKind::Shell),
            &[host(true)]
        ));
        let mut local = ended(Some(255), SessionKind::Shell);
        local.remote = None;
        assert!(!connection_lost(&local, &plain));
        let mut live = ended(Some(255), SessionKind::Shell);
        live.lifecycle = Lifecycle::Running;
        assert!(!connection_lost(&live, &plain));
    }
}
