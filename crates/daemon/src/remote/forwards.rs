//! Local port forwards on a project's OpenSSH master. Tracking is in memory:
//! a forward lives exactly as long as the master that carries it, so status
//! re-checks the master with `ssh -O check` and drops forwards whose master
//! is gone.
use super::*;
use std::{collections::HashSet, sync::Arc};
use terminator_core::remote::{ActiveForward, Forward, ForwardFailure, ForwardStatus};

/// How long an `auto_start` waits for a new terminal to sign in and become
/// (or join) the master before giving up.
const AUTO_START_WAIT: Duration = Duration::from_mins(2);
const CHECK_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub project: String,
    pub forward: Forward,
    pub control: PathBuf,
}

/// Pure bookkeeping; every OpenSSH call happens outside its lock.
#[derive(Default)]
pub(crate) struct Forwards {
    active: Vec<Entry>,
    failures: Vec<ForwardFailure>,
    /// Masters this daemon started only to carry forwards. They persist
    /// without an idle timeout, so the daemon exits them when their last
    /// forward stops.
    dedicated: HashSet<PathBuf>,
}
impl Forwards {
    pub(crate) fn find(&self, project: &str, local_port: u16) -> Option<&Entry> {
        self.active
            .iter()
            .find(|e| e.project == project && e.forward.local_port == local_port)
    }
    /// Another project's forward already listening on `local_port`.
    pub(crate) fn other_owner(&self, project: &str, local_port: u16) -> Option<&Entry> {
        self.active
            .iter()
            .find(|e| e.project != project && e.forward.local_port == local_port)
    }
    pub(crate) fn insert(&mut self, entry: Entry) {
        self.forget_failure(&entry.project, entry.forward.local_port);
        self.active.retain(|e| {
            !(e.project == entry.project && e.forward.local_port == entry.forward.local_port)
        });
        self.active.push(entry);
    }
    pub(crate) fn remove(&mut self, project: &str, local_port: u16) -> Option<Entry> {
        self.forget_failure(project, local_port);
        let index = self
            .active
            .iter()
            .position(|e| e.project == project && e.forward.local_port == local_port)?;
        Some(self.active.remove(index))
    }
    pub(crate) fn fail(&mut self, project: &str, local_port: u16, message: String) {
        self.forget_failure(project, local_port);
        self.failures.push(ForwardFailure {
            project: project.to_owned(),
            local_port,
            message,
        });
    }
    fn forget_failure(&mut self, project: &str, local_port: u16) {
        self.failures
            .retain(|f| !(f.project == project && f.local_port == local_port));
    }
    pub(crate) fn mark_dedicated(&mut self, control: PathBuf) {
        self.dedicated.insert(control);
    }
    /// Whether `control` is a dedicated master that no longer carries any
    /// forward; it is then forgotten so it is exited at most once.
    pub(crate) fn release_dedicated(&mut self, control: &Path) -> bool {
        !self.active.iter().any(|e| e.control == control) && self.dedicated.remove(control)
    }
    pub(crate) fn controls(&self) -> Vec<PathBuf> {
        let mut controls: Vec<_> = self.active.iter().map(|e| e.control.clone()).collect();
        controls.extend(self.dedicated.iter().cloned());
        controls.sort();
        controls.dedup();
        controls
    }
    /// Drops forwards (and dedicated markers) whose master is not alive.
    pub(crate) fn prune(&mut self, alive: impl Fn(&Path) -> bool) {
        self.active.retain(|e| alive(&e.control));
        self.dedicated.retain(|c| alive(c));
    }
    /// Removes everything about `project`, returning its active forwards.
    pub(crate) fn remove_project(&mut self, project: &str) -> Vec<Entry> {
        self.failures.retain(|f| f.project != project);
        let (gone, kept) = std::mem::take(&mut self.active)
            .into_iter()
            .partition(|e| e.project == project);
        self.active = kept;
        gone
    }
    /// Accepts a replacement list for `project` only if every active forward
    /// keeps its endpoints; `auto_start` may change. Failures for removed
    /// forwards are dropped.
    pub(crate) fn redefine(&mut self, project: &str, forwards: &[Forward]) -> Result<()> {
        for entry in self.active.iter().filter(|e| e.project == project) {
            ensure!(
                forwards.iter().any(|f| f.same_endpoints(&entry.forward)),
                "Stop the forward on local port {} before changing or removing it",
                entry.forward.local_port
            );
        }
        for entry in self.active.iter_mut().filter(|e| e.project == project) {
            if let Some(forward) = forwards.iter().find(|f| f.same_endpoints(&entry.forward)) {
                entry.forward = forward.clone();
            }
        }
        self.failures.retain(|f| {
            f.project != project || forwards.iter().any(|g| g.local_port == f.local_port)
        });
        Ok(())
    }
    pub(crate) fn status(&self) -> ForwardStatus {
        let mut active: Vec<_> = self
            .active
            .iter()
            .map(|e| ActiveForward {
                project: e.project.clone(),
                forward: e.forward.clone(),
            })
            .collect();
        active.sort_by(|a, b| {
            (&a.project, a.forward.local_port).cmp(&(&b.project, b.forward.local_port))
        });
        ForwardStatus {
            active,
            failures: self.failures.clone(),
        }
    }
}

/// `ssh -O check`, bounded. A missing socket is down without spawning.
fn alive(control: &Path) -> bool {
    control.exists()
        && run_command(
            terminator_core::ssh::check_control(control),
            CommandOptions {
                timeout: CHECK_TIMEOUT,
                stdout_limit: 4096,
                stderr_limit: 4096,
                ..CommandOptions::default()
            },
        )
        .is_ok()
}

/// Whether `port` is free on the loopback address forwards bind.
fn local_port_free(port: u16) -> bool {
    std::net::TcpListener::bind((terminator_core::remote::BIND_ADDRESS, port)).is_ok()
}

/// Starts a dedicated batch master. Authentication must come from a key or
/// agent; otherwise the classified `AuthFailed` tells the GUI to sign in
/// through an SSH terminal first.
fn start_master(project: &RemoteProject, control: &Path) -> Result<()> {
    let mut command = Command::new("ssh");
    command.args(terminator_core::ssh::forward_master_arguments(
        &project.ssh,
        control,
    )?);
    for key in terminator_core::ssh::private_environment_keys() {
        command.env_remove(key);
    }
    command.arg("--").arg(&project.ssh.host);
    // `-f` detaches the master after authentication and points its stdio
    // at /dev/null, so the pipes close and this returns.
    let output = run_command(
        command,
        CommandOptions {
            timeout: Duration::from_secs(30),
            stdout_limit: 4096,
            stderr_limit: STDERR_LIMIT,
            accepted_exit_codes: None,
            ..CommandOptions::default()
        },
    )
    .context("Cannot start the SSH connection for port forwarding")?;
    checked(output.status, &output.stderr)
}

fn forward_command(start: bool, control: &Path, forward: &Forward) -> Result<()> {
    let output = run_command(
        terminator_core::ssh::forward_control(start, control, forward)?,
        CommandOptions {
            timeout: Duration::from_secs(10),
            stdout_limit: 4096,
            stderr_limit: STDERR_LIMIT,
            accepted_exit_codes: None,
            ..CommandOptions::default()
        },
    )?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    // `ssh -O cancel` reports failures on stderr yet may exit 0.
    ensure!(
        output.status.success() && !stderr.contains("request failed"),
        "{}",
        stderr.trim()
    );
    Ok(())
}

impl Shared {
    fn forward_target(
        &self,
        id: &str,
        local_port: u16,
    ) -> Result<(RemoteProject, Forward, PathBuf)> {
        let project = relock(&self.state)
            .remote_projects
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .context("Unknown SSH project")?;
        let forward = project
            .forwards
            .iter()
            .find(|f| f.local_port == local_port)
            .cloned()
            .with_context(|| format!("No saved forward on local port {local_port}"))?;
        let control = self.ssh_control(&project.ssh).context(
            "Port forwarding needs SSH connection sharing, which this data directory's path is too long for",
        )?;
        Ok((project, forward, control))
    }

    /// Re-checks every master carrying a forward and reports what is left.
    pub(crate) fn forward_status(&self) -> ForwardStatus {
        let controls = relock(&self.forwards).controls();
        let dead: HashSet<PathBuf> = controls.into_iter().filter(|c| !alive(c)).collect();
        let mut forwards = relock(&self.forwards);
        forwards.prune(|c| !dead.contains(c));
        forwards.status()
    }

    /// `allow_master` starts a dedicated master when none answers; the
    /// terminal auto-start path instead waits for the terminal's own.
    fn start_forward_inner(&self, id: &str, local_port: u16, allow_master: bool) -> Result<()> {
        let (project, forward, control) = self.forward_target(id, local_port)?;
        // Lock order is state, then forwards; never the reverse.
        let owner = relock(&self.forwards)
            .other_owner(id, local_port)
            .map(|e| e.project.clone());
        if let Some(owner) = owner {
            let name = relock(&self.state)
                .remote_projects
                .iter()
                .find(|p| p.id == owner)
                .map_or(owner.clone(), |p| p.name.clone());
            anyhow::bail!("Local port {local_port} is already forwarded by SSH project {name}");
        }
        let tracked = relock(&self.forwards)
            .find(id, local_port)
            .is_some_and(|e| e.forward.same_endpoints(&forward) && e.control == control);
        let up = alive(&control);
        if tracked && up {
            return Ok(());
        }
        if !up {
            ensure!(
                allow_master,
                "The SSH terminal has not signed in, so the forward was not started"
            );
            start_master(&project, &control)?;
            relock(&self.forwards).mark_dedicated(control.clone());
        }
        // A port busy before the request is reported as such. The request
        // still goes out when the port is busy and the master was already up:
        // OpenSSH accepts a duplicate of a forward it already carries (one an
        // earlier daemon generation started), and refuses anything else.
        let free = local_port_free(local_port);
        ensure!(
            free || up,
            "Local port {local_port} is already in use on this computer. Stop whatever listens there or choose another local port."
        );
        forward_command(true, &control, &forward).map_err(|error| {
            if free && local_port_free(local_port) {
                anyhow::anyhow!("Cannot forward localhost:{local_port}: {error:#}")
            } else {
                anyhow::anyhow!(
                    "Local port {local_port} is already in use on this computer. Stop whatever listens there or choose another local port."
                )
            }
        })?;
        relock(&self.forwards).insert(Entry {
            project: id.to_owned(),
            forward,
            control,
        });
        Ok(())
    }

    pub(crate) fn start_forward(&self, id: &str, local_port: u16) -> Result<ForwardStatus> {
        self.start_forward_inner(id, local_port, true)?;
        Ok(self.forward_status())
    }

    pub(crate) fn stop_forward(&self, id: &str, local_port: u16) -> Result<ForwardStatus> {
        let tracked = relock(&self.forwards).remove(id, local_port);
        let (forward, control) = if let Some(entry) = tracked {
            (entry.forward, entry.control)
        } else {
            let (_, forward, control) = self.forward_target(id, local_port)?;
            (forward, control)
        };
        if alive(&control) {
            // An untracked forward (an earlier daemon generation started it)
            // reports "port not forwarded" when it is already gone; either
            // way it no longer listens.
            let _ = forward_command(false, &control, &forward);
        }
        self.release_forward_master(&control);
        Ok(self.forward_status())
    }

    /// Exits a dedicated master once its last forward stopped, unless a live
    /// terminal or VNC session joined it meanwhile.
    fn release_forward_master(&self, control: &Path) {
        if !relock(&self.forwards).release_dedicated(control) {
            return;
        }
        let state = relock(&self.state);
        let uses = |ssh: &terminator_core::ssh::SshOptions| {
            self.ssh_control(ssh).as_deref() == Some(control)
        };
        let busy = state
            .sessions
            .iter()
            .filter(|s| s.lifecycle.live())
            .filter_map(|s| s.remote.as_ref())
            .any(|r| uses(&r.ssh))
            || state.vnc_sessions.iter().any(|s| {
                s.status.live()
                    && s.options.transport == terminator_core::vnc::Transport::Ssh
                    && uses(&s.options.ssh)
            });
        drop(state);
        if !busy {
            stop_control(control);
        }
    }

    /// Cancels `project`'s forwards before it is forgotten; best-effort.
    pub(crate) fn drop_project_forwards(&self, project: &str) {
        let gone = relock(&self.forwards).remove_project(project);
        for entry in gone {
            if let Ok(command) =
                terminator_core::ssh::forward_control(false, &entry.control, &entry.forward)
            {
                spawn_bounded(command, "ssh-forward-cancel");
            }
        }
    }

    /// After a terminal for `project` opened: once its SSH connection signs
    /// in (or joins a live master), starts every `auto_start` forward that is
    /// not already up. Never starts a separate master.
    pub(crate) fn auto_start_forwards(self: &Arc<Self>, project: &str) {
        let wanted: Vec<u16> = relock(&self.state)
            .remote_projects
            .iter()
            .find(|p| p.id == project)
            .map(|p| {
                p.forwards
                    .iter()
                    .filter(|f| f.auto_start)
                    .map(|f| f.local_port)
                    .collect()
            })
            .unwrap_or_default();
        if wanted.is_empty() {
            return;
        }
        let shared = Arc::clone(self);
        let project = project.to_owned();
        let _ = std::thread::Builder::new()
            .name("ssh-forward-auto".into())
            .spawn(move || {
                let Some(control) = relock(&shared.state)
                    .remote_projects
                    .iter()
                    .find(|p| p.id == project)
                    .and_then(|p| shared.ssh_control(&p.ssh))
                else {
                    return;
                };
                let deadline = std::time::Instant::now()
                    .checked_add(AUTO_START_WAIT)
                    .unwrap_or_else(std::time::Instant::now);
                while !alive(&control) {
                    if std::time::Instant::now() >= deadline
                        || shared.shutdown.load(std::sync::atomic::Ordering::Acquire)
                    {
                        break;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
                for port in wanted {
                    if let Err(error) = shared.start_forward_inner(&project, port, false) {
                        relock(&shared.forwards).fail(&project, port, format!("{error:#}"));
                    }
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn forward(local_port: u16, auto_start: bool) -> Forward {
        Forward {
            local_port,
            remote_host: "localhost".into(),
            remote_port: 80,
            auto_start,
        }
    }
    fn entry(project: &str, local_port: u16, control: &str) -> Entry {
        Entry {
            project: project.into(),
            forward: forward(local_port, false),
            control: control.into(),
        }
    }

    #[test]
    fn forwards_disappear_from_status_when_their_master_dies() {
        let mut forwards = Forwards::default();
        forwards.insert(entry("a", 8080, "/run/ssh-a"));
        forwards.insert(entry("b", 9090, "/run/ssh-b"));
        forwards.mark_dedicated("/run/ssh-b".into());
        assert_eq!(forwards.controls().len(), 2);
        forwards.prune(|c| c == Path::new("/run/ssh-a"));
        let status = forwards.status();
        assert!(status.is_active("a", 8080));
        assert!(!status.is_active("b", 9090));
        assert!(!forwards.release_dedicated(Path::new("/run/ssh-b")));
    }

    #[test]
    fn starting_again_replaces_and_clears_the_failure() {
        let mut forwards = Forwards::default();
        forwards.fail("a", 8080, "in use".into());
        assert_eq!(forwards.status().failure("a", 8080), Some("in use"));
        forwards.insert(entry("a", 8080, "/run/ssh-a"));
        forwards.insert(entry("a", 8080, "/run/ssh-a"));
        let status = forwards.status();
        assert_eq!(status.active.len(), 1);
        assert!(status.failures.is_empty());
        assert!(forwards.other_owner("b", 8080).is_some());
        assert!(forwards.other_owner("a", 8080).is_none());
    }

    #[test]
    fn a_dedicated_master_is_released_only_after_its_last_forward() {
        let mut forwards = Forwards::default();
        let control = Path::new("/run/ssh-a");
        forwards.mark_dedicated(control.into());
        forwards.insert(entry("a", 1, "/run/ssh-a"));
        forwards.insert(entry("a", 2, "/run/ssh-a"));
        forwards.remove("a", 1);
        assert!(!forwards.release_dedicated(control));
        forwards.remove("a", 2);
        assert!(forwards.release_dedicated(control));
        assert!(!forwards.release_dedicated(control));
    }

    #[test]
    fn redefining_keeps_active_endpoints_and_allows_auto_start_changes() {
        let mut forwards = Forwards::default();
        forwards.insert(entry("a", 8080, "/run/ssh-a"));
        forwards.fail("a", 9090, "x".into());
        assert!(forwards.redefine("a", &[]).is_err());
        let mut moved = forward(8080, false);
        moved.remote_port = 81;
        assert!(forwards.redefine("a", &[moved]).is_err());
        forwards.redefine("a", &[forward(8080, true)]).unwrap();
        let status = forwards.status();
        assert!(status.active.first().unwrap().forward.auto_start);
        assert!(status.failures.is_empty());
        assert!(forwards.redefine("b", &[]).is_ok());
        assert_eq!(forwards.remove_project("a").len(), 1);
        assert!(forwards.status().active.is_empty());
    }
}
