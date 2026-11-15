//! Listening ports of every session of a project, for the Ports tool. One
//! process table and one `lsof` call cover all sessions; each port is
//! attributed to the session whose process tree owns it.
use super::*;
#[cfg(any(feature = "async-client", test))]
use std::collections::HashMap;

/// Upper bound on the PIDs handed to one `lsof` call.
#[cfg(any(feature = "async-client", test))]
const MAX_PIDS: usize = 2048;

/// A listening port and the session process tree that owns it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionPort {
    pub session: String,
    pub pid: u32,
    /// Process start time, checked again before Stop so a reused PID is
    /// never signalled.
    pub started: u64,
    pub command: String,
    pub address: String,
    pub port: u16,
}

impl SessionPort {
    #[must_use]
    pub fn url(&self) -> String {
        ListeningPort {
            pid: self.pid,
            address: self.address.clone(),
            port: self.port,
        }
        .url()
    }
}

#[cfg(any(feature = "async-client", test))]
#[derive(Clone, Debug, PartialEq, Eq)]
struct Owner {
    session: String,
    started: u64,
    command: String,
}

/// Every process of each session, keyed by PID. A session whose process
/// ended, was replaced, or has too many descendants is skipped.
#[cfg(any(feature = "async-client", test))]
fn owners(tree: &ProcessTree, sessions: &[(String, (u32, u64))]) -> HashMap<u32, Owner> {
    let mut owners = HashMap::new();
    for (session, identity) in sessions {
        let Ok(owned) = tree.owned(*identity) else {
            continue;
        };
        for pid in owned {
            if owners.len() >= MAX_PIDS {
                return owners;
            }
            let process = tree.system.process(sysinfo::Pid::from_u32(pid));
            owners.entry(pid).or_insert_with(|| Owner {
                session: session.clone(),
                started: process.map_or(0, sysinfo::Process::start_time),
                command: process
                    .map(|p| p.name().to_string_lossy().chars().take(64).collect())
                    .unwrap_or_default(),
            });
        }
    }
    owners
}

#[cfg(any(feature = "async-client", test))]
fn attribute(lsof: &str, owners: &HashMap<u32, Owner>) -> Vec<SessionPort> {
    let owned = owners.keys().copied().collect::<HashSet<_>>();
    parse_ports(lsof, &owned)
        .into_iter()
        .filter_map(|port| {
            let owner = owners.get(&port.pid)?;
            Some(SessionPort {
                session: owner.session.clone(),
                pid: port.pid,
                started: owner.started,
                command: owner.command.clone(),
                address: port.address,
                port: port.port,
            })
        })
        .collect()
}

/// Listening TCP ports of `sessions` (session id, (PID, start time)).
#[cfg(feature = "async-client")]
pub async fn session_ports(
    processes: &crate::async_process::Processes,
    files: &crate::async_service::NativePool,
    sessions: Vec<(String, (u32, u64))>,
) -> Result<Vec<SessionPort>> {
    let owners = files
        .run(&crate::async_service::CancellationToken::new(), move || {
            Ok::<_, MetadataError>(owners(&ProcessTree::new(process_table()), &sessions))
        })
        .await?;
    if owners.is_empty() {
        return Ok(Vec::new());
    }
    let mut pids = owners.keys().copied().collect::<Vec<_>>();
    pids.sort_unstable();
    let pids = pids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let stdout = listening_sockets(processes, &pids).await?;
    Ok(attribute(&String::from_utf8_lossy(&stdout), &owners))
}

/// Sends the polite stop signal (SIGTERM on Unix) to `pid`, but only while
/// it is still the process that was listed: same PID and start time.
pub fn stop_port_process(pid: u32, started: u64) -> Result<()> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
        true,
        ProcessRefreshKind::nothing(),
    );
    let process = system
        .process(Pid::from_u32(pid))
        .filter(|p| p.start_time().abs_diff(started) <= 2)
        .ok_or(MetadataError::ProcessGone { pid })?;
    let sent = process
        .kill_with(sysinfo::Signal::Term)
        .unwrap_or_else(|| process.kill());
    if sent {
        Ok(())
    } else {
        Err(MetadataError::SignalFailed { pid })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(session: &str) -> Owner {
        Owner {
            session: session.into(),
            started: 7,
            command: "node".into(),
        }
    }

    #[test]
    fn each_port_goes_to_the_session_that_owns_its_process() {
        let owners = HashMap::from([(10, owner("s1")), (20, owner("s2"))]);
        let lsof = "p10\nn*:3000\np20\nn127.0.0.1:8080\np30\nn*:9999\n";
        let ports = attribute(lsof, &owners);
        let found = ports
            .iter()
            .map(|p| (p.session.as_str(), p.port, p.pid))
            .collect::<Vec<_>>();
        assert_eq!(found, [("s1", 3000, 10), ("s2", 8080, 20)]);
        assert_eq!(ports[0].url(), "http://127.0.0.1:3000/");
        assert_eq!(ports[0].command, "node");
    }

    #[test]
    fn the_current_process_tree_is_found_and_a_wrong_start_time_is_skipped() {
        let tree = ProcessTree::new(process_table());
        let me = std::process::id();
        let started = tree
            .system
            .process(sysinfo::Pid::from_u32(me))
            .map(sysinfo::Process::start_time)
            .unwrap();
        let found = owners(&tree, &[("s".into(), (me, started))]);
        assert_eq!(found.get(&me).map(|o| o.session.as_str()), Some("s"));
        let stale = owners(&tree, &[("s".into(), (me, started + 1000))]);
        assert!(stale.is_empty());
    }

    #[test]
    fn stop_refuses_a_process_whose_start_time_changed() {
        let me = std::process::id();
        assert!(matches!(
            stop_port_process(me, 1),
            Err(MetadataError::ProcessGone { .. })
        ));
    }
}
