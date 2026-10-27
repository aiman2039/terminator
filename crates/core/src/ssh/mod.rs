//! Shared OpenSSH configuration. Keys and agents remain owned by OpenSSH.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

mod config;
pub use config::{ConfigHost, ConfigSource, config_hosts, parse_config_aliases, parse_config_host};

/// `ScanHostKey`, `TrustHostKey` and `Response::SshError`.
pub const HOST_KEYS_CAPABILITY: &str = "ssh-hostkeys-v1";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SshOptions {
    pub host: String,
    /// `None` passes no `-p`, so `~/.ssh/config` (or 22) decides. Omitted
    /// when unset so older daemons fall back to their own default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    pub username: String,
    pub identity_file: Option<PathBuf>,
    pub known_hosts_file: Option<PathBuf>,
}
impl SshOptions {
    pub fn validate(&self) -> Result<()> {
        validate_host(&self.host)?;
        ensure!(self.port != Some(0), "SSH port must be between 1 and 65535");
        ensure!(
            self.username.len() <= 128
                && !self.username.starts_with('-')
                && self
                    .username
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c)),
            "Invalid SSH username"
        );
        for path in [&self.identity_file, &self.known_hosts_file]
            .into_iter()
            .flatten()
        {
            ensure!(path.is_absolute(), "SSH file paths must be absolute");
            ensure!(
                path.to_str()
                    .is_some_and(|s| s.len() <= 4096 && !s.chars().any(char::is_control)),
                "SSH paths must be valid text without control characters"
            );
        }
        Ok(())
    }
    /// `user@host:port` for display, omitting whatever OpenSSH configuration
    /// decides.
    #[must_use]
    pub fn authority(&self) -> String {
        let mut text = String::new();
        if !self.username.is_empty() {
            text.push_str(&self.username);
            text.push('@');
        }
        text.push_str(&self.host);
        if let Some(port) = self.port {
            text.push_str(&format!(":{port}"));
        }
        text
    }
}

/// `control` enables OpenSSH multiplexing through that socket; see
/// `Paths::ssh_control`. Without it every call opens its own connection.
pub fn arguments(
    options: &SshOptions,
    terminal: bool,
    control: Option<&Path>,
) -> Result<Vec<OsString>> {
    build(options, terminal, control.map(|path| (path, false)))
}

/// Batch arguments for a dedicated multiplexing master that only carries
/// port forwards: `-f -N` backgrounds it after authentication, and it
/// persists until `ssh -O exit` rather than for an idle timeout, because
/// idle forward listeners do not count as activity for `ControlPersist`.
///
/// `ClearAllForwardings=yes` stays: OpenSSH applies it once, while
/// finalizing the master's own options, to forwards from configuration and
/// its command line. Forwards added later through `ssh -O forward` are mux
/// requests the master processes on its own and are unaffected (verified
/// against OpenSSH 10.3). It is the *requesting* client that must not carry
/// it, or that client clears its own `-L` before sending; see
/// [`forward_control`].
pub fn forward_master_arguments(options: &SshOptions, control: &Path) -> Result<Vec<OsString>> {
    let mut args = build(options, false, Some((control, true)))?;
    args.extend(["-f".into(), "-N".into()]);
    Ok(args)
}

fn build(
    options: &SshOptions,
    terminal: bool,
    control: Option<(&Path, bool)>,
) -> Result<Vec<OsString>> {
    options.validate()?;
    // A terminal has a PTY, so OpenSSH may prompt there for passwords,
    // passphrases, 2FA and first-connect host keys. Batch calls never prompt;
    // with `control` they reuse the master a terminal authenticated.
    let (tty, strict) = if terminal {
        ("-tt", "-oStrictHostKeyChecking=ask")
    } else {
        ("-T", "-oStrictHostKeyChecking=yes")
    };
    let mut args: Vec<OsString> = [
        tty,
        "-oForwardAgent=no",
        "-oForwardX11=no",
        "-oPermitLocalCommand=no",
        strict,
        "-oClearAllForwardings=yes",
        "-oConnectTimeout=10",
        "-oServerAliveInterval=15",
        "-oServerAliveCountMax=3",
        "-oLogLevel=ERROR",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    if !terminal {
        args.push("-oBatchMode=yes".into());
    }
    if let Some(port) = options.port {
        args.extend([OsString::from("-p"), port.to_string().into()]);
    }
    if !options.username.is_empty() {
        args.extend([OsString::from("-l"), options.username.clone().into()]);
    }
    if let Some(path) = &options.identity_file {
        args.extend([OsString::from("-i"), path.as_os_str().into()]);
    }
    if let Some(path) = &options.known_hosts_file {
        args.extend([
            OsString::from("-o"),
            format!("UserKnownHostsFile={}", quoted(path)).into(),
        ]);
    }
    if let Some((path, dedicated)) = control {
        let (master, persist) = if dedicated {
            ("-oControlMaster=yes", "-oControlPersist=yes")
        } else {
            ("-oControlMaster=auto", "-oControlPersist=10m")
        };
        args.extend([
            master.into(),
            // ControlPath expands `%` tokens; a literal runtime path must not.
            format!("-oControlPath={}", quoted(path).replace('%', "%%")).into(),
            persist.into(),
        ]);
    } else {
        args.push("-oControlMaster=no".into());
    }
    Ok(args)
}

/// Asks the multiplexing master behind `control` to exit. Best-effort: the
/// master may already be gone. `-F none` keeps user configuration out of it.
pub fn exit_control(control: &Path) -> std::process::Command {
    control_command("exit", control, None)
}

/// Exits 0 only while a multiplexing master answers on `control`.
pub fn check_control(control: &Path) -> std::process::Command {
    control_command("check", control, None)
}

/// `ssh -O forward` (`start`) or `ssh -O cancel` for one local forward on
/// the master behind `control`. `-F none` keeps `LocalForward` entries from
/// `~/.ssh/config` out of the request, and no `ClearAllForwardings` is
/// passed: on this client it would drop the `-L` before it is sent.
pub fn forward_control(
    start: bool,
    control: &Path,
    forward: &crate::remote::Forward,
) -> Result<std::process::Command> {
    forward.validate()?;
    Ok(control_command(
        if start { "forward" } else { "cancel" },
        control,
        Some(&forward.spec()),
    ))
}

fn control_command(operation: &str, control: &Path, local: Option<&str>) -> std::process::Command {
    let mut command = std::process::Command::new("ssh");
    command.args(["-F", "none", "-O", operation]).arg(format!(
        "-oControlPath={}",
        quoted(control).replace('%', "%%")
    ));
    if let Some(spec) = local {
        command.arg("-L").arg(spec);
    }
    command.args(["--", "terminator-control"]);
    for key in private_environment_keys() {
        command.env_remove(key);
    }
    command
}

/// Why an OpenSSH call failed, derived from its stderr.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SshFailure {
    UnknownHostKey,
    ChangedHostKey,
    AuthFailed,
    Unreachable,
    Other(String),
}

/// Classifies OpenSSH client stderr. A changed key is checked first: its
/// warning also ends with "Host key verification failed.".
#[must_use]
pub fn classify(stderr: &str) -> SshFailure {
    let has = |needle: &str| stderr.contains(needle);
    if has("REMOTE HOST IDENTIFICATION HAS CHANGED")
        || has("has changed and you have requested strict checking")
    {
        SshFailure::ChangedHostKey
    } else if has("Host key verification failed.") || has(" host key is known for ") {
        SshFailure::UnknownHostKey
    } else if has("Permission denied (") || has("Too many authentication failures") {
        SshFailure::AuthFailed
    } else if [
        "Connection refused",
        "Could not resolve hostname",
        "Connection timed out",
        "Operation timed out",
        "No route to host",
        "Network is unreachable",
    ]
    .iter()
    .any(|needle| has(needle))
    {
        SshFailure::Unreachable
    } else {
        SshFailure::Other(stderr.trim().to_owned())
    }
}

/// A classified OpenSSH failure. Daemons send it as `Response::SshError`
/// to clients that opt in; `Response::checked` turns it back into this type
/// so callers can `downcast_ref`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshError {
    pub failure: SshFailure,
    pub message: String,
}
impl std::fmt::Display for SshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for SshError {}
impl SshError {
    /// `None` when stderr is not a recognized SSH connection failure.
    #[must_use]
    pub fn from_stderr(stderr: &[u8]) -> Option<Self> {
        let text = String::from_utf8_lossy(stderr);
        let failure = classify(&text);
        let summary = match failure {
            SshFailure::UnknownHostKey => "The SSH host key is not trusted yet",
            SshFailure::ChangedHostKey => "The SSH host key changed",
            SshFailure::AuthFailed => "SSH authentication failed",
            SshFailure::Unreachable => "The SSH host is unreachable",
            SshFailure::Other(_) => return None,
        };
        Some(Self {
            failure,
            message: format!("{summary}: {}", text.trim()),
        })
    }
}

/// One public host key as `ssh-keyscan` reports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostKey {
    pub key_type: String,
    /// Base64 public key blob.
    pub key: String,
    /// `SHA256:…` from `ssh-keygen -l`. Informational; never trusted.
    #[serde(default)]
    pub fingerprint: String,
}

/// Keys a host presented, plus the `known_hosts` host field they would be
/// stored under.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostKeyScan {
    pub host: String,
    pub keys: Vec<HostKey>,
}

/// Endpoint after `ssh -G` applied the user's configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub hostname: String,
    pub port: u16,
    pub host_key_alias: Option<String>,
}
impl Resolved {
    /// The host field OpenSSH looks up: a `HostKeyAlias` verbatim, otherwise
    /// the hostname, bracketed with the port when it is not 22.
    #[must_use]
    pub fn known_hosts_host(&self) -> String {
        match &self.host_key_alias {
            Some(alias) => alias.clone(),
            None if self.port == 22 => self.hostname.clone(),
            None => format!("[{}]:{}", self.hostname, self.port),
        }
    }
}

/// Parses `ssh -G` output.
pub fn parse_resolved(output: &str) -> Result<Resolved> {
    let mut hostname = None;
    let mut port = None;
    let mut host_key_alias = None;
    for line in output.lines() {
        let Some((key, value)) = line.split_once(' ') else {
            continue;
        };
        let value = value.trim();
        match key {
            "hostname" => hostname = Some(value.to_owned()),
            "port" => port = value.parse::<u16>().ok(),
            "hostkeyalias" if !value.is_empty() && value != "none" => {
                host_key_alias = Some(value.to_owned());
            }
            _ => {}
        }
    }
    let hostname = hostname.context("ssh -G did not report a hostname")?;
    validate_host(&hostname)?;
    if let Some(alias) = &host_key_alias {
        validate_host(alias)?;
    }
    Ok(Resolved {
        hostname,
        port: port
            .filter(|p| *p > 0)
            .context("ssh -G did not report a port")?,
        host_key_alias,
    })
}

/// Parses `ssh-keyscan` stdout. Comments and malformed lines are skipped;
/// duplicate keys are reported once.
#[must_use]
pub fn parse_keyscan(output: &str) -> Vec<HostKey> {
    let mut keys: Vec<HostKey> = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let (Some(_host), Some(key_type), Some(key)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let valid_type = key_type.len() <= 64
            && key_type
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-@.".contains(c));
        let valid_key = !key.is_empty()
            && key.len() <= 16 * 1024
            && key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+/=".contains(c));
        if valid_type && valid_key && !keys.iter().any(|k| k.key_type == key_type && k.key == key) {
            keys.push(HostKey {
                key_type: key_type.to_owned(),
                key: key.to_owned(),
                fingerprint: String::new(),
            });
        }
    }
    keys
}

/// Parses `ssh-keygen -l` lines (`256 SHA256:… comment (ED25519)`), in order.
#[must_use]
pub fn parse_fingerprints(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .filter(|fingerprint| fingerprint.starts_with("SHA256:"))
        .map(str::to_owned)
        .collect()
}

/// `~/.ssh/known_hosts`, where OpenSSH looks first without configuration.
#[must_use]
pub fn default_known_hosts() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|base| base.home_dir().join(".ssh").join("known_hosts"))
}

/// One `known_hosts` line, newline-terminated.
#[must_use]
pub fn known_hosts_line(host: &str, key: &HostKey) -> String {
    format!("{host} {} {}\n", key.key_type, key.key)
}

/// Whether two scans present the same keys, ignoring order and fingerprints.
#[must_use]
pub fn same_keys(a: &[HostKey], b: &[HostKey]) -> bool {
    let set = |keys: &[HostKey]| {
        let mut set: Vec<(String, String)> = keys
            .iter()
            .map(|k| (k.key_type.clone(), k.key.clone()))
            .collect();
        set.sort();
        set.dedup();
        set
    };
    !a.is_empty() && set(a) == set(b)
}

fn quoted(path: &Path) -> String {
    format!(
        "\"{}\"",
        path.to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
    )
}

/// Local daemon capabilities must never reach a remote host through `SendEnv`.
pub fn private_environment_keys() -> Vec<OsString> {
    std::env::vars_os()
        .filter_map(|(key, _)| {
            key.to_string_lossy()
                .to_ascii_uppercase()
                .starts_with("TERMINATOR_")
                .then_some(key)
        })
        .collect()
}

pub fn validate_host(host: &str) -> Result<()> {
    ensure!(
        !host.is_empty()
            && host.len() <= 255
            && !host.starts_with('-')
            && host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || ".-_:".contains(c)),
        "Invalid host name or IP address"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ssh_arguments_require_trust_and_disable_implicit_forwarding() {
        let config = SshOptions {
            host: "fixture".into(),
            username: "developer".into(),
            ..SshOptions::default()
        };
        let args = arguments(&config, false, None).unwrap();
        for argument in [
            "-T",
            "-oBatchMode=yes",
            "-oStrictHostKeyChecking=yes",
            "-oForwardAgent=no",
            "-oPermitLocalCommand=no",
            "-oClearAllForwardings=yes",
            "-oControlMaster=no",
        ] {
            assert!(args.contains(&OsString::from(argument)));
        }
        assert!(
            !args
                .iter()
                .any(|a| a.to_string_lossy().starts_with("-oControlPath"))
        );
    }
    #[test]
    fn ssh_terminals_may_prompt_but_keep_forwarding_disabled() {
        let config = SshOptions {
            host: "fixture".into(),
            ..SshOptions::default()
        };
        let args = arguments(&config, true, None).unwrap();
        for argument in [
            "-tt",
            "-oStrictHostKeyChecking=ask",
            "-oForwardAgent=no",
            "-oPermitLocalCommand=no",
            "-oClearAllForwardings=yes",
        ] {
            assert!(args.contains(&OsString::from(argument)), "{argument}");
        }
        assert!(
            !args
                .iter()
                .any(|a| a.to_string_lossy().starts_with("-oBatchMode"))
        );
        assert!(!args.contains(&OsString::from("-oStrictHostKeyChecking=yes")));
    }
    #[test]
    fn forward_master_is_a_batch_master_that_persists_until_exit() {
        let config = SshOptions {
            host: "fixture".into(),
            ..SshOptions::default()
        };
        let control = Path::new("/tmp/terminator-x/ssh-0000abcd");
        let args = forward_master_arguments(&config, control).unwrap();
        for argument in [
            "-T",
            "-f",
            "-N",
            "-oBatchMode=yes",
            "-oStrictHostKeyChecking=yes",
            "-oClearAllForwardings=yes",
            "-oControlMaster=yes",
            "-oControlPersist=yes",
        ] {
            assert!(args.contains(&OsString::from(argument)), "{argument}");
        }
        assert!(!args.contains(&OsString::from("-oControlMaster=auto")));
        assert!(!args.contains(&OsString::from("-oControlPersist=10m")));
        assert!(
            !args
                .iter()
                .any(|a| a.to_string_lossy().starts_with("-L") || a.to_string_lossy() == "-g")
        );
    }
    #[test]
    fn forward_control_sends_a_loopback_only_local_forward() {
        let control = Path::new("/tmp/terminator-x/ssh-0000abcd");
        let forward = crate::remote::Forward {
            local_port: 8080,
            remote_host: "db".into(),
            remote_port: 5432,
            auto_start: false,
        };
        for (start, operation) in [(true, "forward"), (false, "cancel")] {
            let command = forward_control(start, control, &forward).unwrap();
            let args: Vec<_> = command
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            assert_eq!(args.get(..4).unwrap(), ["-F", "none", "-O", operation]);
            let at = args.iter().position(|a| a == "-L").unwrap();
            assert_eq!(args.get(at + 1).unwrap(), "127.0.0.1:8080:db:5432");
            assert!(!args.iter().any(|a| a.contains("ClearAllForwardings")));
            assert!(
                !args
                    .iter()
                    .any(|a| a.contains("0.0.0.0") || a.starts_with('*'))
            );
            assert_eq!(args.last().unwrap(), "terminator-control");
        }
        let bad = crate::remote::Forward {
            remote_host: "-oProxyCommand=x".into(),
            ..forward
        };
        assert!(forward_control(true, control, &bad).is_err());
    }
    #[test]
    fn openssh_stderr_is_classified() {
        for (stderr, expected) in [
            (
                "No ED25519 host key is known for fixture and you have requested strict checking.\r\nHost key verification failed.\r\n",
                SshFailure::UnknownHostKey,
            ),
            (
                "Host key verification failed.\n",
                SshFailure::UnknownHostKey,
            ),
            (
                "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\n@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @\n@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\nIT IS POSSIBLE THAT SOMEONE IS DOING SOMETHING NASTY!\nHost key for fixture has changed and you have requested strict checking.\nHost key verification failed.\n",
                SshFailure::ChangedHostKey,
            ),
            (
                "developer@fixture: Permission denied (publickey,password).\r\n",
                SshFailure::AuthFailed,
            ),
            (
                "Received disconnect from 10.0.0.2 port 22:2: Too many authentication failures\n",
                SshFailure::AuthFailed,
            ),
            (
                "ssh: connect to host fixture port 2222: Connection refused\r\n",
                SshFailure::Unreachable,
            ),
            (
                "ssh: Could not resolve hostname nowhere.invalid: nodename nor servname provided, or not known\r\n",
                SshFailure::Unreachable,
            ),
            (
                "ssh: connect to host 10.255.255.1 port 22: Connection timed out\r\n",
                SshFailure::Unreachable,
            ),
            (
                "ssh: connect to host 10.255.255.1 port 22: Operation timed out\r\n",
                SshFailure::Unreachable,
            ),
            (
                "  sh: git: command not found\n",
                SshFailure::Other("sh: git: command not found".into()),
            ),
        ] {
            assert_eq!(classify(stderr), expected, "{stderr}");
        }
        assert!(SshError::from_stderr(b"sh: git: command not found").is_none());
        let error = SshError::from_stderr(b"Host key verification failed.\n").unwrap();
        assert_eq!(error.failure, SshFailure::UnknownHostKey);
        assert!(error.message.ends_with("Host key verification failed."));
    }
    #[test]
    fn known_hosts_lines_bracket_non_default_ports() {
        let key = HostKey {
            key_type: "ssh-ed25519".into(),
            key: "AAAAC3NzaC1lZDI1NTE5AAAAIHNvbWUta2V5".into(),
            fingerprint: "SHA256:ignored".into(),
        };
        let mut resolved = Resolved {
            hostname: "fixture.example".into(),
            port: 22,
            host_key_alias: None,
        };
        assert_eq!(
            known_hosts_line(&resolved.known_hosts_host(), &key),
            "fixture.example ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHNvbWUta2V5\n"
        );
        resolved.port = 2222;
        assert_eq!(
            known_hosts_line(&resolved.known_hosts_host(), &key),
            "[fixture.example]:2222 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHNvbWUta2V5\n"
        );
        resolved.host_key_alias = Some("alias".into());
        assert_eq!(resolved.known_hosts_host(), "alias");
    }
    #[test]
    fn keyscan_and_keygen_output_is_parsed() {
        let scan = "# fixture.example:2222 SSH-2.0-OpenSSH_9.6\n\
            [fixture.example]:2222 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHNvbWUta2V5\n\
            [fixture.example]:2222 ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTY=\n\
            [fixture.example]:2222 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHNvbWUta2V5\n\
            malformed\n\
            host bad$type AAAA\n";
        let keys = parse_keyscan(scan);
        assert_eq!(
            keys.iter().map(|k| k.key_type.as_str()).collect::<Vec<_>>(),
            ["ssh-ed25519", "ecdsa-sha2-nistp256"]
        );
        let mut reversed = keys.clone();
        reversed.reverse();
        assert!(same_keys(&keys, &reversed));
        assert!(!same_keys(&keys, &keys[..1]));
        assert!(!same_keys(&[], &[]));
        assert_eq!(
            parse_fingerprints(
                "256 SHA256:uNiVztksCsDhcc0u9e8BujQXVUpKZIDTMczCvj3tD2s fixture (ED25519)\n\
                 256 SHA256:Ab/cD+ef fixture (ECDSA)\n"
            ),
            [
                "SHA256:uNiVztksCsDhcc0u9e8BujQXVUpKZIDTMczCvj3tD2s",
                "SHA256:Ab/cD+ef"
            ]
        );
        let resolved = parse_resolved(
            "user developer\nhostname fixture.example\nport 2222\nhostkeyalias none\n",
        )
        .unwrap();
        assert_eq!(resolved.hostname, "fixture.example");
        assert_eq!(resolved.port, 2222);
        assert_eq!(resolved.host_key_alias, None);
        assert!(parse_resolved("hostname -oProxyCommand=x\nport 22\n").is_err());
    }
    #[test]
    fn saved_ports_stay_explicit_and_missing_ports_defer_to_config() {
        let saved: SshOptions =
            serde_json::from_str(r#"{"host":"build","port":22,"username":"dev"}"#).unwrap();
        assert_eq!(saved.port, Some(22));
        for json in [r#"{"host":"build"}"#, r#"{"host":"build","port":null}"#] {
            let options: SshOptions = serde_json::from_str(json).unwrap();
            assert_eq!(options.port, None, "{json}");
        }
        for options in [
            saved.clone(),
            SshOptions {
                port: None,
                ..saved.clone()
            },
        ] {
            let text = serde_json::to_string(&options).unwrap();
            assert_eq!(text.contains("\"port\""), options.port.is_some());
            assert_eq!(serde_json::from_str::<SshOptions>(&text).unwrap(), options);
        }
        let port = |options: &SshOptions| {
            let args = arguments(options, false, None).unwrap();
            args.iter()
                .position(|a| a == "-p")
                .and_then(|i| args.get(i.checked_add(1)?).cloned())
        };
        assert_eq!(port(&saved), Some(OsString::from("22")));
        assert_eq!(
            port(&SshOptions {
                port: None,
                ..saved.clone()
            }),
            None
        );
        assert!(
            SshOptions {
                port: Some(0),
                ..saved.clone()
            }
            .validate()
            .is_err()
        );
        assert_eq!(saved.authority(), "dev@build:22");
        assert_eq!(
            SshOptions {
                port: None,
                username: String::new(),
                ..saved
            }
            .authority(),
            "build"
        );
    }
    #[test]
    fn ssh_arguments_reuse_the_given_control_socket() {
        let config = SshOptions {
            host: "fixture".into(),
            ..SshOptions::default()
        };
        let args = arguments(&config, false, Some(Path::new("/tmp/t-1/ssh-0a%b"))).unwrap();
        for argument in [
            "-oControlMaster=auto",
            "-oControlPath=\"/tmp/t-1/ssh-0a%%b\"",
            "-oControlPersist=10m",
            "-oClearAllForwardings=yes",
        ] {
            assert!(args.contains(&OsString::from(argument)), "{argument}");
        }
        assert!(!args.contains(&OsString::from("-oControlMaster=no")));
    }
    #[test]
    fn control_sockets_fit_unix_limits_and_separate_endpoints() {
        let paths = crate::Paths {
            data: "/unused".into(),
            runtime: "/tmp/terminator-ffffffffffffffff".into(),
        };
        let long = SshOptions {
            host: "h".repeat(255),
            username: "u".repeat(128),
            identity_file: Some(PathBuf::from(format!("/{}", "k".repeat(4000)))),
            ..SshOptions::default()
        };
        let control = paths.ssh_control(&long).unwrap();
        assert!(control.as_os_str().len() < 100 - 17);
        assert_eq!(paths.ssh_control(&long), Some(control.clone()));
        let other = SshOptions {
            port: Some(2222),
            ..long.clone()
        };
        assert_ne!(paths.ssh_control(&other), Some(control));
        let deep = crate::Paths {
            data: "/unused".into(),
            runtime: PathBuf::from(format!("/{}", "r".repeat(80))),
        };
        assert_eq!(deep.ssh_control(&long), None);
    }
    #[test]
    fn control_check_and_exit_target_only_the_socket() {
        let path = Path::new("/tmp/t/ssh-%h");
        for (command, op) in [(check_control(path), "check"), (exit_control(path), "exit")] {
            let args: Vec<_> = command.get_args().collect();
            assert_eq!(
                args,
                [
                    "-F",
                    "none",
                    "-O",
                    op,
                    "-oControlPath=\"/tmp/t/ssh-%%h\"",
                    "--",
                    "terminator-control"
                ]
            );
        }
    }
}
