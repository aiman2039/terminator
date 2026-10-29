//! Detects hosts that `~/.ssh/config` configures, so a new VNC host can
//! default to an SSH tunnel. OpenSSH resolves the config (`ssh -G`); a
//! host counts as configured when its resolved settings differ from those
//! of a name no config matches. Runs off the GUI thread.
use std::{
    process::{Command, Stdio},
    sync::mpsc::{Receiver, TryRecvError, channel},
    time::{Duration, Instant},
};

/// Wait after the last keystroke before probing.
const DEBOUNCE: Duration = Duration::from_millis(400);
/// A name no `Host` pattern should match except `*`.
const BASELINE_HOST: &str = "terminator-ssh-probe.invalid";

/// What `~/.ssh/config` sets for a host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SshHostConfig {
    pub(crate) host: String,
    pub(crate) hostname: String,
    pub(crate) user: Option<String>,
    pub(crate) port: Option<u16>,
    pub(crate) identity_file: Option<String>,
}

impl SshHostConfig {
    /// One-line description for the notice under the form.
    pub(crate) fn summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(user) = &self.user {
            parts.push(format!("user {user}"));
        }
        if let Some(port) = self.port {
            parts.push(format!("port {port}"));
        }
        if let Some(key) = &self.identity_file {
            let name = key.rsplit('/').next().unwrap_or(key);
            parts.push(format!("key {name}"));
        }
        if parts.is_empty() {
            format!("Found {} in ~/.ssh/config. Using an SSH tunnel.", self.host)
        } else {
            format!(
                "Found {} in ~/.ssh/config ({}). Using an SSH tunnel.",
                self.host,
                parts.join(", ")
            )
        }
    }
}

/// Debounced, single-flight probe for the host typed into a form.
#[derive(Default)]
pub(crate) struct SshProbe {
    /// Host and when it last changed, waiting for the debounce.
    pending: Option<(String, Instant)>,
    running: Option<(String, Receiver<Option<SshHostConfig>>)>,
    /// Last host probed, so an unchanged field is not probed again.
    probed: String,
}

impl SshProbe {
    /// Note the current host field. Starts a probe once it has been stable
    /// for [`DEBOUNCE`]; returns a finished result for the current host.
    pub(crate) fn poll(&mut self, host: &str) -> Option<SshHostConfig> {
        let host = host.trim();
        let finished = self.finished(host);
        if host == self.probed || !plausible_host(host) {
            self.pending = None;
            return finished;
        }
        match &self.pending {
            Some((pending, since)) if pending == host => {
                if since.elapsed() >= DEBOUNCE && self.running.is_none() {
                    self.start(host.to_owned());
                }
            }
            _ => self.pending = Some((host.to_owned(), Instant::now())),
        }
        finished
    }

    /// True while a probe is waiting or running; the caller keeps
    /// repainting so the debounce fires without input.
    pub(crate) fn busy(&self) -> bool {
        self.pending.is_some() || self.running.is_some()
    }

    fn finished(&mut self, host: &str) -> Option<SshHostConfig> {
        let (probed, receiver) = self.running.as_ref()?;
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => None,
        };
        let probed = probed.clone();
        self.running = None;
        result.filter(|_| probed == host)
    }

    fn start(&mut self, host: String) {
        self.pending = None;
        self.probed.clone_from(&host);
        let (sender, receiver) = channel();
        let probe_host = host.clone();
        let spawned = std::thread::Builder::new()
            .name("vnc-ssh-probe".into())
            .spawn(move || {
                let _ = sender.send(probe(&probe_host));
            });
        if spawned.is_ok() {
            self.running = Some((host, receiver));
        }
    }
}

/// Host names `ssh` accepts as an argument: no options, spaces or users.
fn plausible_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 255
        && !host.starts_with('-')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-_".contains(c))
}

fn probe(host: &str) -> Option<SshHostConfig> {
    let resolved = resolve(host)?;
    let baseline = resolve(BASELINE_HOST)?;
    configured(host, &resolved, &baseline)
}

/// `ssh -G host` as key/value pairs. `None` when ssh is missing or fails.
fn resolve(host: &str) -> Option<Vec<(String, String)>> {
    let output = Command::new("ssh")
        .args(["-G", "--", host])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output.status.success().then(|| {
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.split_once(' '))
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect()
    })
}

fn value<'a>(config: &'a [(String, String)], key: &str) -> Option<&'a str> {
    config
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn identity_files(config: &[(String, String)]) -> Vec<&str> {
    config
        .iter()
        .filter(|(k, _)| k == "identityfile")
        .map(|(_, v)| v.as_str())
        .collect()
}

/// Compare a host's resolved settings with the baseline. The baseline's
/// hostname is itself, so only a different hostname counts there.
fn configured(
    host: &str,
    resolved: &[(String, String)],
    baseline: &[(String, String)],
) -> Option<SshHostConfig> {
    let hostname = value(resolved, "hostname").unwrap_or(host);
    let differs = |key: &str| value(resolved, key) != value(baseline, key);
    let keys = identity_files(resolved);
    let custom_key = keys != identity_files(baseline);
    let matched =
        !hostname.eq_ignore_ascii_case(host) || differs("user") || differs("port") || custom_key;
    matched.then(|| SshHostConfig {
        host: host.to_owned(),
        hostname: hostname.to_owned(),
        user: differs("user")
            .then(|| value(resolved, "user").map(str::to_owned))
            .flatten(),
        port: differs("port")
            .then(|| value(resolved, "port").and_then(|p| p.parse().ok()))
            .flatten(),
        identity_file: custom_key
            .then(|| keys.first().map(|k| (*k).to_owned()))
            .flatten(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(text: &str) -> Vec<(String, String)> {
        text.lines()
            .filter_map(|l| l.split_once(' '))
            .map(|(k, v)| (k.into(), v.into()))
            .collect()
    }

    #[test]
    fn a_host_with_its_own_config_block_is_detected() {
        let baseline = pairs(
            "hostname terminator-ssh-probe.invalid\nuser me\nport 22\nidentityfile ~/.ssh/id_rsa",
        );
        let resolved =
            pairs("hostname build.example.com\nuser me\nport 22\nidentityfile ~/.ssh/id_ed25519");
        let found = configured("build.example.com", &resolved, &baseline).unwrap();
        assert_eq!(found.identity_file.as_deref(), Some("~/.ssh/id_ed25519"));
        assert_eq!(found.user, None);
        assert!(found.summary().contains("key id_ed25519"));
    }

    #[test]
    fn a_host_only_matched_by_wildcards_is_not_detected() {
        let baseline = pairs("hostname terminator-ssh-probe.invalid\nuser me\nport 22");
        let resolved = pairs("hostname plain.example.com\nuser me\nport 22");
        assert!(configured("plain.example.com", &resolved, &baseline).is_none());
    }

    #[test]
    fn option_like_or_spaced_hosts_are_never_probed() {
        for host in ["-oProxyCommand=x", "a b", "user@host", ""] {
            assert!(!plausible_host(host), "{host}");
        }
        assert!(plausible_host("dc6-container-xterm-17.nvidia.com"));
    }
}
