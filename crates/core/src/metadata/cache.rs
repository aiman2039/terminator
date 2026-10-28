//! Per-terminal metadata cache. Git facts refresh every 30 s, the pull
//! request every minute (or when the branch changes), ports on every call.
use super::*;

#[derive(Clone)]
pub struct Cache {
    pub(super) cwd: Option<PathBuf>,
    pub(super) data: Metadata,
    pub(super) git_at: Instant,
    pub(super) pr_at: Instant,
    pub(super) pr_enabled: bool,
}
impl Default for Cache {
    fn default() -> Self {
        Self {
            cwd: None,
            data: Metadata::default(),
            git_at: minute_ago(),
            pr_at: minute_ago(),
            pr_enabled: false,
        }
    }
}

fn minute_ago() -> Instant {
    Instant::now()
        .checked_sub(Duration::from_mins(1))
        .unwrap_or_else(Instant::now)
}

impl Cache {
    pub fn collect(
        &mut self,
        cwd: &Path,
        identity: Option<(u32, u64)>,
        include_pr: bool,
    ) -> Metadata {
        let changed = self.enter(cwd);
        if self.git_due(changed) {
            let previous = self.data.branch.clone();
            self.data.root = git(cwd, &["rev-parse", "--show-toplevel"])
                .ok()
                .map(PathBuf::from);
            self.data.branch = git(cwd, &["branch", "--show-current"])
                .ok()
                .filter(|s| !s.is_empty());
            self.data.worktree = self
                .data
                .root
                .as_ref()
                .is_some_and(|p| p.join(".git").is_file());
            self.branch_checked(previous);
        }
        if self.pr_due(include_pr, changed) {
            let pr = pull_request(cwd);
            self.store_pr(pr);
        }
        self.finish_pr(include_pr);
        let ports = identity.map(listening_ports).transpose();
        self.store_ports(ports)
    }

    /// Starts over when the terminal moved to another directory.
    pub(super) fn enter(&mut self, cwd: &Path) -> bool {
        let changed = self.cwd.as_deref() != Some(cwd);
        if changed {
            self.cwd = Some(cwd.into());
            self.data = Metadata {
                cwd: cwd.into(),
                ..Default::default()
            };
        }
        changed
    }

    /// Whether Git facts are stale; restarts their timer when they are.
    pub(super) fn git_due(&mut self, changed: bool) -> bool {
        let due = changed || self.git_at.elapsed() >= Duration::from_secs(30);
        if due {
            self.git_at = Instant::now();
        }
        due
    }

    /// A new branch makes the pull request stale at once.
    pub(super) fn branch_checked(&mut self, previous: Option<String>) {
        if self.data.branch != previous {
            self.pr_at = minute_ago();
        }
    }

    /// Whether the pull request is wanted and stale; restarts its timer when it is.
    pub(super) fn pr_due(&mut self, include_pr: bool, changed: bool) -> bool {
        let due = include_pr
            && self.data.root.is_some()
            && (changed || !self.pr_enabled || self.pr_at.elapsed() >= Duration::from_mins(1));
        if due {
            self.pr_at = Instant::now();
        }
        due
    }

    pub(super) fn store_pr(&mut self, pr: Result<Option<PullRequest>>) {
        match pr {
            Ok(pr) => {
                self.data.pull_request = pr;
                self.data.pr_error = None;
            }
            Err(error) => {
                self.data.pull_request = None;
                self.data.pr_error = Some(error.to_string().chars().take(256).collect());
            }
        }
    }

    pub(super) fn finish_pr(&mut self, include_pr: bool) {
        if !include_pr {
            self.data.pull_request = None;
            self.data.pr_error = None;
        }
        self.pr_enabled = include_pr;
    }

    pub(super) fn store_ports(&mut self, ports: Result<Option<Vec<ListeningPort>>>) -> Metadata {
        match ports {
            Ok(ports) => {
                self.data.ports = ports.unwrap_or_default();
                self.data.ports_error = None;
            }
            Err(error) => {
                self.data.ports.clear();
                self.data.ports_error = Some(error.to_string());
            }
        }
        self.data.clone()
    }
}
